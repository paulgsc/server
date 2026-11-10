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
//! in `leetype`, and `GET /api/v1/leetype/session-files` serves them. A week
//! of Rust Rounds rides `brief` as `rust-week-<id>.json`, its programs queued
//! for the recorder as it is kept, and `GET /api/v1/rust-weeks` serves it.
//!
//! **When a pass runs** (MBP2, paulgsc/server#500). This repo leans against
//! cron-shaped design (`nudge::waker`'s header): a timer should pick up work
//! that something already decided, not go looking for it. The mailbox cannot
//! call in, because home makes only outbound connections, so home opens one
//! and holds it: the mailbox's bell ([`bell`]), which rings when the routine
//! puts to a folder home reads. A pass runs on a ring, on each (re)connect
//! (to catch up on what was put while home wasn't listening), as soon as home
//! files something for the mailbox itself ([`Outbox`]), and on the backstop
//! timer, every `mailbox::INTERVAL` (an hour; 15 minutes after a pass that
//! failed, [`RETRY`]), which keeps every file arriving while the bell is
//! down. One loop runs every
//! pass, so two never overlap, and causes raised during a pass fold into at
//! most one more. A pass is idempotent (cursor plus SHA-256), so an extra one
//! costs a few listings and nothing else.
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
//!   of and never archives. The weekly call's index is the same kind of view,
//!   of `call_week` ([`index::call`]), the weeks [`crate::call_weeks`] keeps.
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
//! **The arrival notice** (`jobs_notice`). With `FCM_SERVICE_ACCOUNT_FILE`,
//! each pass is followed by one of [`crate::jobs_notice::run_once`], which
//! tells the phones that asked when a morning run's record has arrived.

pub mod bell;
pub mod index;
pub mod pass;

use crate::auth::owner::{configured_or_owner, Owner};
use crate::auth::DeletionLock;
use crate::{AppState, Config};
use axum::extract::FromRef;
use mailbox_kit::{Mailbox, ReqwestTransport, Token};
use sqlx::SqlitePool;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};

/// The shortest interval a pass may run at.
pub const MIN_INTERVAL: Duration = Duration::from_secs(60);

/// The backstop: an hour.
///
/// Home reads the mailbox at least this often, whatever the bell does. A
/// source constant (#617). It was 15 minutes while the timer was the only
/// way a pass ran; the owner chose an hour once the bell landed (2026-11-10,
/// #500), which keeps MI6's idle bound at 24 passes a day plus one per
/// reconnect.
pub const INTERVAL: Duration = Duration::from_secs(3600);

/// How soon the timer runs again after a pass that failed.
///
/// 15 minutes, the old interval, so `MailboxPollerFailing`'s 45 minutes is still three failed
/// passes in a row rather than one failure held for an hour.
pub const RETRY: Duration = Duration::from_secs(900);

/// How long a `LeetType` session waits before a pass files its log.
///
/// Its newest event must have been home 15 minutes, so a send still in flight is never filed
/// in part. It was the poll interval, and stays what that was.
pub const SETTLE: Duration = Duration::from_secs(900);

/// How often the idle pass loop says it is alive
/// (`mailbox_poll_alive_timestamp_seconds`), so a dead or hung loop is heard
/// in minutes, not after an hour-long interval (MI3).
pub const HEARTBEAT: Duration = Duration::from_secs(60);

/// How often the idle pass loop stamps the routine watch.
///
/// [`crate::routine_watch::stamp`] runs between passes. Each pass stamps it too,
/// but passes are hourly when nothing rings, and RR2 promises a missing
/// run is told 30 minutes after it was due: the due time must move by then.
pub const WATCH: Duration = Duration::from_secs(300);

const _: () = assert!(INTERVAL.as_secs() >= MIN_INTERVAL.as_secs());
const _: () = assert!(RETRY.as_secs() >= MIN_INTERVAL.as_secs() && RETRY.as_secs() <= INTERVAL.as_secs());

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
	/// What each routine's last index the mailbox answered was computed
	/// from, so a pass puts one only when that moves ([`index::LastIndexes`]).
	/// Memory only: home keeps no copy of an index.
	pub last_index: Arc<index::LastIndexes>,
	/// What wakes the pass loop besides its timer: the bell and [`Outbox`].
	pub wake: Arc<bell::Wake>,
	/// The mailbox's bell, through `MAILBOX_TOKEN`.
	pub bell: Arc<bell::Bell>,
}

/// Wakes the mailbox's pass loop when home files something for the mailbox,
/// so it goes out in seconds rather than at the next timer pass. Does nothing
/// when the mailbox is off.
#[derive(Clone, Default)]
pub struct Outbox(Option<Arc<bell::Wake>>);

impl Outbox {
	/// An outbox that raises on `wake`, for a test's state.
	#[cfg(test)]
	pub(crate) const fn waking(wake: Arc<bell::Wake>) -> Self {
		Self(Some(wake))
	}

	/// Something was filed for the mailbox: run a pass soon.
	pub fn filed(&self) {
		if let Some(wake) = &self.0 {
			wake.raise(bell::Cause::Local);
		}
	}
}

impl FromRef<AppState> for Outbox {
	fn from_ref(state: &AppState) -> Self {
		Self(state.mailbox.as_ref().map(|context| Arc::clone(&context.wake)))
	}
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
	/// host, `mailbox::INTERVAL` below 60, or a `MAILBOX_ARCHIVE_TOKEN` that is
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
		let interval = INTERVAL;
		let archive_token = set(config.mailbox_archive_token.as_ref());
		if archive_token.as_ref() == Some(&token) {
			anyhow::bail!("MAILBOX_ARCHIVE_TOKEN must not be MAILBOX_TOKEN");
		}
		if archive_token.is_some() && !(MIN_ARCHIVE_DAYS..=MAX_ARCHIVE_DAYS).contains(&config.mailbox_archive_days) {
			anyhow::bail!("MAILBOX_ARCHIVE_DAYS must be from 30 to 36500");
		}
		let client = reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build()?;
		let bell = bell::Bell::new(&url, Token::new(token.clone()), bell::Timings::new(INTERVAL)).map_err(|why| anyhow::anyhow!("MAILBOX_URL: {why}"))?;
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
			wake: Arc::default(),
			bell: Arc::new(bell),
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

/// What a pass did and why it ran, by counts and kinds only: no file's name
/// or body, so no transcript's text, reaches the log.
pub(crate) fn log_pass(result: &Result<pass::Outcome, pass::PassError>, cause: bell::Cause) {
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
					cause = cause.name(),
					"Jobs mailbox pass"
				);
			}
		}
		Err(err) => {
			for (phase, failure) in &err.failures {
				error!(error = %failure, phase = phase.name(), class = failure.class(), cause = cause.name(), "Jobs mailbox pass failed");
			}
		}
	}
}

/// Spawn the pass loop and the bell, both cancelled through the shared
/// token. The first pass runs at once, so a restart catches up without
/// waiting an interval.
fn spawn(state: &AppState, context: MailboxContext) {
	let cancel = state.core.cancel_token.clone();
	let db = state.core.shared_db.clone();
	crate::metrics::mailbox::record_interval(context.interval);

	tokio::spawn(bell::hold(Arc::clone(&context.bell), Arc::clone(&context.wake), cancel.clone()));
	tokio::spawn(async move {
		info!(interval_secs = context.interval.as_secs(), "Jobs mailbox poller started");
		let wake = Arc::clone(&context.wake);
		run_passes(&wake, context.interval, RETRY, &cancel, |cause| tick(&db, &context, cause), || watch(&db, &context)).await;
		info!("Jobs mailbox poller cancelled");
	});
}

/// The one pass loop: a pass at once, then one per cause, until `cancel`.
///
/// Passes never overlap: this loop is the only caller of `pass`. Causes
/// raised while a pass runs fold into at most one more. Each pass restarts
/// the backstop timer, so a pass a ring ran is not followed by the timer's a
/// moment later: `interval` after a pass that succeeded (`pass` returns
/// true), `retry` after one that failed. While idle the loop stamps that it
/// is alive every [`HEARTBEAT`], and runs `watch` every [`WATCH`]; during a
/// pass it does neither, so a hung pass reads as stale, and `watch` never
/// races a pass's own stamp.
async fn run_passes<F, Fut, W, WFut>(wake: &bell::Wake, interval: Duration, retry: Duration, cancel: &tokio_util::sync::CancellationToken, mut pass: F, mut watch: W)
where
	F: FnMut(bell::Cause) -> Fut,
	Fut: std::future::Future<Output = bool>,
	W: FnMut() -> WFut,
	WFut: std::future::Future<Output = ()>,
{
	let mut ticker = tokio::time::interval(interval);
	ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
	let mut heartbeat = tokio::time::interval(HEARTBEAT);
	heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
	// The first pass stamps the watch; this one starts a period later.
	let mut watching = tokio::time::interval_at(tokio::time::Instant::now() + WATCH, WATCH);
	watching.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
	loop {
		let cause = tokio::select! {
			() = cancel.cancelled() => return,
			_ = heartbeat.tick() => {
				crate::metrics::mailbox::record_alive();
				continue;
			}
			_ = watching.tick() => {
				watch().await;
				continue;
			}
			// Whatever was raised before the timer fired rides its pass.
			_ = ticker.tick() => wake.take().unwrap_or(bell::Cause::Timer),
			cause = wake.wait() => cause,
		};
		crate::metrics::mailbox::record_pass_cause(cause);
		let succeeded = pass(cause).await;
		crate::metrics::mailbox::record_alive();
		if succeeded {
			ticker.reset();
		} else {
			ticker.reset_after(retry);
		}
	}
}

/// Between passes: stamp the routine watch for the pass's subject, so a
/// routine that was due moves its gauge without waiting for the hourly pass.
/// Reads home's own tables only; no request to the mailbox.
async fn watch(db: &SqlitePool, context: &MailboxContext) {
	if context.schedules.is_empty() {
		return;
	}
	match pass_subject(db, context.subject.as_deref()).await {
		Ok(Owner::Subject(subject)) => crate::routine_watch::stamp(db, &subject, &context.schedules, chrono::Utc::now()).await,
		Ok(Owner::NoAccount | Owner::Several) => {}
		Err(err) => error!(error = %err, "could not read whose routines to watch"),
	}
}

/// One pass of the poller: find whose copy to keep, run a pass for it, then
/// stamp the routine watch and tell the phones. True when it succeeded: no
/// phase failed, and there was an account to keep the files for.
async fn tick(db: &SqlitePool, context: &MailboxContext, cause: bell::Cause) -> bool {
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
			return false;
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
				SETTLE,
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
	log_pass(&result, cause);
	let succeeded = result.as_ref().is_ok_and(|outcome| !outcome.no_account);
	let Some(subject) = subject else {
		return succeeded;
	};
	crate::routine_watch::stamp(db, &subject, &context.schedules, chrono::Utc::now()).await;
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
	succeeded
}
