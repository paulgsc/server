//! The Jobs mailbox, read from home.
//!
//! The owner's cloud routine writes each day's job digest at 07:52 Pacific.
//! The phone takes it from home (`GET /api/v1/jobs/digests`), and home is
//! LAN-only, so the routine cannot hand it over directly. The two meet at a
//! mailbox, a Cloudflare Worker over a D1 database (paulgsc/some-ui
//! `apps/mailbox`): the routine puts the digest in its `brief` folder, and
//! this module reads `brief` and keeps a copy of each file in
//! `mailbox_item` (`mailbox_repo`), which the route serves. The reflections
//! routine's word sets and dramas arrive the same way, in `scene` and
//! `drama`, with a record of each of its runs in `run`, and
//! `GET /api/v1/routine/files` serves them. `LeetType`'s session files arrive
//! in `leetype`, and `GET /api/v1/leetype/session-files` serves them.
//!
//! **Why this is a poll.** This repo leans against cron-shaped design
//! (`nudge::waker`'s header): a timer should pick up work that something
//! already decided, not go looking for it. Here nothing on home's side can
//! learn that a file arrived, because the mailbox cannot call in: home makes
//! only outbound requests. So it asks, every `MAILBOX_SECONDS` (900 by
//! default), and an idle pass is one listing request for items after the
//! highest `seq` home holds, which answers nothing. Asking more often makes a
//! digest reach home sooner; it changes nothing else.
//!
//! What this module promises, as docs/identity.md invariant 19 states:
//!
//! - It reads only the folders in [`pass::READS`] (`brief`, `scene`, `drama`,
//!   `run` and `leetype`), for one subject, the owner's account unless
//!   `MAILBOX_SUBJECT` names another ([`pass_subject`]), and writes only
//!   `mailbox_item`, through [`pass::run_once`].
//! - It puts only what is filed for the mailbox: the phone's decisions,
//!   scouts and opened lessons, each transcript home heard in a voice note
//!   (`transcript`), text only, and each finished `LeetType` session's log
//!   (`leetype-log`), with no soundbite, both of which it files itself, and
//!   the Jobs routine's index (`index`), its bookkeeping computed from those
//!   same tables ([`index`]): a view, not a record, which home keeps no copy
//!   of and never archives.
//! - It keeps only bytes that match their listing's SHA-256 and parse as
//!   JSON, and holds `auth::DeletionLock` across each write, which checks the
//!   subject's account in the same statement (invariant 9). The lock is
//!   never held across a request to the mailbox.
//! - The token is a `mailbox_kit::Token`, which never prints, and errors are
//!   logged by kind: no token, URL or body reaches a log line.
//!
//! **Archiving** (docs/identity.md invariant 21). The mailbox is where the
//! routine and home meet, not the record, so with `MAILBOX_ARCHIVE_TOKEN` a
//! pass also moves each file live there for `MAILBOX_ARCHIVE_DAYS` into
//! `mailbox_archive`, then has the mailbox drop its bytes, and puts an
//! archived file back when the routine asks (`pass`'s module doc).
//!
//! **Expected runs** (`routine_watch`). After each pass, home says when each
//! routine in `ROUTINE_SCHEDULE` was last due and when its newest run started,
//! so an alert can tell a run that never came from a quiet morning.
//!
//! **Unanswered** (AS7, paulgsc/server#525). After each pass, home says since
//! when the oldest ask it left open and the oldest file it filed that the
//! mailbox has not answered have waited (`stamp_unanswered`), so an alert can
//! tell either from a box that is merely busy.
//!
//! **The arrival notice** (`jobs_notice`). With `FCM_SERVICE_ACCOUNT_FILE`,
//! each pass is followed by one of [`crate::jobs_notice::run_once`], which
//! tells the phones that asked when a morning run's record has arrived.

pub mod index;
pub mod pass;

use crate::auth::owner::{configured_or_owner, Owner};
use crate::auth::DeletionLock;
use crate::{AppState, Config};
use mailbox_kit::{Mailbox, ReqwestTransport, Token};
use sqlx::SqlitePool;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};

/// The shortest interval `MAILBOX_SECONDS` may set.
pub const MIN_INTERVAL: Duration = Duration::from_secs(60);

/// The fewest days `MAILBOX_ARCHIVE_DAYS` may set: the mailbox's own floor
/// (`ARCHIVE_MIN_AGE_DAYS` in paulgsc/some-ui `apps/mailbox/src/core.ts`),
/// below which it refuses every archive.
pub const MIN_ARCHIVE_DAYS: u64 = 30;

/// The most days `MAILBOX_ARCHIVE_DAYS` may set: a hundred years, so the age
/// a pass counts back to is always a date it can name.
pub const MAX_ARCHIVE_DAYS: u64 = 36_500;

/// How long one request to the mailbox may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Everything the poller needs, or `None` when the deployment has not turned
/// the mailbox on.
#[derive(Clone)]
pub struct MailboxContext {
	pub mailbox: Arc<Mailbox<ReqwestTransport>>,
	/// `MAILBOX_SUBJECT`: the subject whose copy the files are kept as, when
	/// the deployment set one. `None` means the owner's account, read at each
	/// pass ([`pass_subject`]).
	pub subject: Option<String>,
	pub interval: Duration,
	/// Account deletion's lock, shared with `AppState::auth`.
	pub deletion: DeletionLock,
	/// The mailbox through the archive token, or `None` when the deployment
	/// archives nothing.
	pub archive: Option<Arc<Mailbox<ReqwestTransport>>>,
	/// Days a file stays live in the mailbox before home archives it.
	pub archive_days: u64,
	/// FCM, as the service account `FCM_SERVICE_ACCOUNT_FILE` names, or
	/// `None` when the deployment tells no phone anything.
	pub notice: Option<Arc<fcm_kit::Sender<fcm_kit::ReqwestTransport>>>,
	/// When each routine is due (`ROUTINE_SCHEDULE`), for
	/// [`crate::routine_watch::stamp`] after each pass.
	pub schedules: Vec<crate::routine_watch::Schedule>,
	/// What the last Jobs index the mailbox answered was computed from, so a
	/// pass puts one only when that moves ([`index::LastIndex`]). Memory only:
	/// home keeps no copy of an index.
	pub last_index: Arc<index::LastIndex>,
}

impl MailboxContext {
	/// Validate the mailbox's configuration once, at startup.
	///
	/// Off is fine and logged. On with anything missing or unusable is a hard
	/// error, as `NUDGE_ENABLED` without its keys is: the only symptom of a
	/// mailbox that never reads is a phone whose picks never arrive. The error
	/// names the variable, never its value.
	///
	/// # Errors
	/// `MAILBOX_ENABLED` with `MAILBOX_URL` or `MAILBOX_TOKEN` unset or
	/// empty, a URL that is not `https://` and a
	/// host, `MAILBOX_SECONDS` below 60, or a `MAILBOX_ARCHIVE_TOKEN` that is
	/// `MAILBOX_TOKEN` again or comes with `MAILBOX_ARCHIVE_DAYS` below 30,
	/// an `FCM_SERVICE_ACCOUNT_FILE` that is not a service account's key, or a
	/// `ROUTINE_SCHEDULE` that does not parse.
	pub fn from_config(config: &Config, deletion: DeletionLock) -> anyhow::Result<Option<Self>> {
		if !config.mailbox_enabled {
			info!("the Jobs mailbox is not enabled; home will not read digests from it");
			if config.fcm_service_account_file.is_some() {
				info!("FCM_SERVICE_ACCOUNT_FILE is set, but with the mailbox off no run arrives to tell a phone of");
			}
			return Ok(None);
		}
		let set = |value: Option<&String>| value.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty());
		let Some(url) = set(config.mailbox_url.as_ref()) else {
			anyhow::bail!("MAILBOX_ENABLED is set but MAILBOX_URL is not");
		};
		let Some(token) = set(config.mailbox_token.as_ref()) else {
			anyhow::bail!("MAILBOX_ENABLED is set but MAILBOX_TOKEN is not");
		};
		let subject = set(config.mailbox_subject.as_ref());
		if subject.is_none() {
			info!("MAILBOX_SUBJECT is unset; the mailbox's files are kept as the owner's account's, the only one");
		}
		let schedules = crate::routine_watch::parse_schedules(&config.routine_schedule).map_err(|why| anyhow::anyhow!("ROUTINE_SCHEDULE: {why}"))?;
		let interval = Duration::from_secs(config.mailbox_seconds);
		if interval < MIN_INTERVAL {
			anyhow::bail!("MAILBOX_SECONDS must be at least 60");
		}
		let archive_token = set(config.mailbox_archive_token.as_ref());
		if archive_token.as_ref() == Some(&token) {
			anyhow::bail!("MAILBOX_ARCHIVE_TOKEN must not be MAILBOX_TOKEN");
		}
		if archive_token.is_some() && !(MIN_ARCHIVE_DAYS..=MAX_ARCHIVE_DAYS).contains(&config.mailbox_archive_days) {
			anyhow::bail!("MAILBOX_ARCHIVE_DAYS must be from 30 to 36500");
		}
		let client = reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build()?;
		let mailbox = Mailbox::new(&url, Token::new(token), ReqwestTransport::new(client.clone())).map_err(|why| anyhow::anyhow!("MAILBOX_URL: {why}"))?;
		let archive = archive_token
			.map(|token| Mailbox::new(&url, Token::new(token), ReqwestTransport::new(client.clone())).map(Arc::new))
			.transpose()
			.map_err(|why| anyhow::anyhow!("MAILBOX_URL: {why}"))?;
		if archive.is_none() {
			info!("MAILBOX_ARCHIVE_TOKEN is not set; home will archive nothing from the mailbox");
		}
		let notice = if let Some(path) = &config.fcm_service_account_file {
			let account = crate::jobs_notice::load(path)?;
			Some(Arc::new(fcm_kit::Sender::new(account, fcm_kit::ReqwestTransport::new(client))))
		} else {
			info!("FCM_SERVICE_ACCOUNT_FILE is not set; home will tell no phone when a run arrives");
			None
		};
		Ok(Some(Self {
			mailbox: Arc::new(mailbox),
			subject,
			interval,
			deletion,
			archive,
			archive_days: config.mailbox_archive_days,
			notice,
			schedules,
			last_index: Arc::default(),
		}))
	}
}

/// Whose copy a pass keeps.
///
/// `MAILBOX_SUBJECT` when set, else the owner's account (`auth::owner`), read
/// now so an account made after boot is found without a restart.
/// `Owner::NoAccount` and `Owner::Several` mean a pass has no subject and
/// keeps nothing (`no_account`).
///
/// # Errors
/// A storage failure, when the owner had to be read.
pub async fn pass_subject(db: &SqlitePool, configured: Option<&str>) -> Result<Owner, sqlx::Error> {
	configured_or_owner(db, configured).await
}

/// Start the poller if the deployment configured one, and say which on
/// `mailbox_poll_enabled` either way.
pub fn start(state: &AppState) {
	crate::metrics::mailbox::record_enabled(state.mailbox.is_some(), state.mailbox.as_ref().is_some_and(|context| context.archive.is_some()));
	if let Some(context) = state.mailbox.clone() {
		spawn(state, context);
	}
}

/// What a pass did, by counts and kinds only: no file's name or body, so no
/// transcript's text, reaches the log.
pub(crate) fn log_pass(result: &Result<pass::Outcome, pass::PassError>) {
	match result {
		Ok(outcome) => {
			let busy = outcome.kept > 0
				|| !outcome.refused.is_empty()
				|| outcome.filed > 0
				|| !outcome.handed.is_empty()
				|| !outcome.indexed.is_empty()
				|| !outcome.archived.is_empty()
				|| !outcome.restored.is_empty();
			if busy || outcome.no_account {
				info!(
					kept = outcome.kept,
					refused = ?outcome.refused,
					filed = outcome.filed,
					handed = ?outcome.handed,
					indexed = ?outcome.indexed,
					archived = ?outcome.archived,
					restored = ?outcome.restored,
					no_account = outcome.no_account,
					"Jobs mailbox pass"
				);
			}
		}
		Err(err) => {
			for (phase, failure) in &err.failures {
				error!(error = %failure, phase = phase.name(), class = failure.class(), "Jobs mailbox pass failed");
			}
		}
	}
}

/// Say since when the oldest ask and the oldest filed file have waited on an
/// answer. The asks are what this pass listed and left open, so a pass whose
/// asks failed leaves that gauge as it was; the filed files are read from
/// home's own tables, whatever the pass did. A read that fails leaves its
/// gauge as it was and says so in the log.
async fn stamp_unanswered(db: &SqlitePool, subject: &str, result: &Result<pass::Outcome, pass::PassError>) {
	let asks = match result {
		Ok(outcome) => Some(outcome),
		Err(err) if !err.failures.iter().any(|(phase, _)| *phase == pass::Phase::Asks) => Some(&err.outcome),
		Err(_) => None,
	};
	if let Some(outcome) = asks {
		crate::metrics::mailbox::record_unanswered("ask", outcome.oldest_open_ask.as_deref());
	}
	let filed = match db.acquire().await {
		Ok(mut conn) => mailbox_repo::oldest_unhanded(&mut conn, subject).await.map_err(|err| err.to_string()),
		Err(err) => Err(err.to_string()),
	};
	match filed {
		Ok(since) => crate::metrics::mailbox::record_unanswered("filed", since.as_deref()),
		Err(err) => error!(error = %err, "could not read the oldest file the mailbox has not answered"),
	}
}

/// Spawn the poller, cancelled through the shared token. Its first pass runs
/// at once, so a restart catches up without waiting an interval.
fn spawn(state: &AppState, context: MailboxContext) {
	let cancel = state.core.cancel_token.clone();
	let db = state.core.shared_db.clone();
	crate::metrics::mailbox::record_interval(context.interval);

	tokio::spawn(async move {
		info!(interval_secs = context.interval.as_secs(), "Jobs mailbox poller started");
		let mut ticker = tokio::time::interval(context.interval);
		ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		loop {
			tokio::select! {
				() = cancel.cancelled() => {
					info!("Jobs mailbox poller cancelled");
					return;
				}
				_ = ticker.tick() => tick(&db, &context).await,
			}
		}
	});
}

/// One tick of the poller: find whose copy to keep, run a pass for it, then
/// stamp the routine watch and tell the phones.
async fn tick(db: &SqlitePool, context: &MailboxContext) {
	crate::metrics::mailbox::record_pass_started();
	let started = tokio::time::Instant::now();
	let subject = match pass_subject(db, context.subject.as_deref()).await {
		Ok(Owner::Subject(subject)) => Some(subject),
		Ok(owner @ (Owner::NoAccount | Owner::Several)) => {
			// Said as a word, never an id, and as a warning only when the owner
			// has something to set: a box waiting for its first passkey is not
			// wrong, a box with several accounts and no `MAILBOX_SUBJECT` is.
			if owner == Owner::Several {
				warn!("the mailbox has no subject: several accounts and MAILBOX_SUBJECT is unset; set it to the owner's subject");
			}
			None
		}
		Err(err) => {
			let outcome = pass::Outcome::default();
			crate::metrics::mailbox::record_pass_duration(started.elapsed());
			crate::metrics::mailbox::record_pass(&outcome, &["storage"]);
			error!(error = %err, phase = "subject", class = "storage", "Jobs mailbox pass failed");
			return;
		}
	};
	let result = match &subject {
		Some(subject) => {
			let archiving = context.archive.as_deref().map(|mailbox| pass::Archiving {
				mailbox,
				after_days: context.archive_days,
			});
			pass::run_once(
				db,
				&context.mailbox,
				archiving.as_ref(),
				subject,
				&context.deletion,
				context.interval,
				&context.last_index,
				&pass::now,
			)
			.await
		}
		None => Ok(pass::Outcome {
			no_account: true,
			..pass::Outcome::default()
		}),
	};
	crate::metrics::mailbox::record_pass_duration(started.elapsed());
	match &result {
		Ok(outcome) => crate::metrics::mailbox::record_pass(outcome, &[]),
		Err(err) => crate::metrics::mailbox::record_pass(&err.outcome, &err.classes()),
	}
	log_pass(&result);
	let Some(subject) = subject else {
		return;
	};
	crate::routine_watch::stamp(db, &subject, &context.schedules, chrono::Utc::now()).await;
	stamp_unanswered(db, &subject, &result).await;
	if let Some(sender) = context.notice.as_deref() {
		match crate::jobs_notice::run_once(db, &context.deletion, &subject, sender, chrono::Utc::now()).await {
			Ok(outcome) if outcome.busy() => info!(
				noticed = outcome.noticed,
				skipped = outcome.skipped,
				forgotten = outcome.forgotten,
				unsent = outcome.unsent,
				"Jobs arrival notice pass"
			),
			Ok(_) => {}
			Err(err) => error!(error = %err, "Jobs arrival notice pass failed"),
		}
	}
}
