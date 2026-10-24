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
//! `drama`, and `GET /api/v1/routine/files` serves them.
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
//! - It reads only the folders in [`pass::READS`] (`brief`, `scene` and
//!   `drama`), for the one subject `MAILBOX_SUBJECT` names, and writes only
//!   `mailbox_item`, through [`pass::run_once`].
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

pub mod pass;

use crate::auth::DeletionLock;
use crate::{AppState, Config};
use mailbox_kit::{Mailbox, ReqwestTransport, Token};
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info};

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
	/// The subject whose copy the files are kept as (`MAILBOX_SUBJECT`).
	pub subject: String,
	pub interval: Duration,
	/// Account deletion's lock, shared with `AppState::auth`.
	pub deletion: DeletionLock,
	/// The mailbox through the archive token, or `None` when the deployment
	/// archives nothing.
	pub archive: Option<Arc<Mailbox<ReqwestTransport>>>,
	/// Days a file stays live in the mailbox before home archives it.
	pub archive_days: u64,
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
	/// `MAILBOX_ENABLED` with `MAILBOX_URL`, `MAILBOX_TOKEN` or
	/// `MAILBOX_SUBJECT` unset or empty, a URL that is not `https://` and a
	/// host, `MAILBOX_SECONDS` below 60, or a `MAILBOX_ARCHIVE_TOKEN` that is
	/// `MAILBOX_TOKEN` again or comes with `MAILBOX_ARCHIVE_DAYS` below 30.
	pub fn from_config(config: &Config, deletion: DeletionLock) -> anyhow::Result<Option<Self>> {
		if !config.mailbox_enabled {
			info!("the Jobs mailbox is not enabled; home will not read digests from it");
			return Ok(None);
		}
		let set = |value: Option<&String>| value.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty());
		let Some(url) = set(config.mailbox_url.as_ref()) else {
			anyhow::bail!("MAILBOX_ENABLED is set but MAILBOX_URL is not");
		};
		let Some(token) = set(config.mailbox_token.as_ref()) else {
			anyhow::bail!("MAILBOX_ENABLED is set but MAILBOX_TOKEN is not");
		};
		let Some(subject) = set(config.mailbox_subject.as_ref()) else {
			anyhow::bail!("MAILBOX_ENABLED is set but MAILBOX_SUBJECT is not");
		};
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
			.map(|token| Mailbox::new(&url, Token::new(token), ReqwestTransport::new(client)).map(Arc::new))
			.transpose()
			.map_err(|why| anyhow::anyhow!("MAILBOX_URL: {why}"))?;
		if archive.is_none() {
			info!("MAILBOX_ARCHIVE_TOKEN is not set; home will archive nothing from the mailbox");
		}
		Ok(Some(Self {
			mailbox: Arc::new(mailbox),
			subject,
			interval,
			deletion,
			archive,
			archive_days: config.mailbox_archive_days,
		}))
	}
}

/// Start the poller if the deployment configured one, and say which on
/// `mailbox_poll_enabled` either way.
pub fn start(state: &AppState) {
	crate::metrics::mailbox::record_enabled(state.mailbox.is_some(), state.mailbox.as_ref().is_some_and(|context| context.archive.is_some()));
	if let Some(context) = state.mailbox.clone() {
		spawn(state, context);
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
				_ = ticker.tick() => {
					crate::metrics::mailbox::record_pass_started();
					let started = tokio::time::Instant::now();
					let archiving = context.archive.as_deref().map(|mailbox| pass::Archiving {
						mailbox,
						after_days: context.archive_days,
					});
					let result = pass::run_once(&db, &context.mailbox, archiving.as_ref(), &context.subject, &context.deletion, &pass::now).await;
					crate::metrics::mailbox::record_pass_duration(started.elapsed());
					match result {
						Ok(outcome) => {
							crate::metrics::mailbox::record_pass(&outcome, None);
							let busy = outcome.kept > 0 || !outcome.refused.is_empty() || !outcome.handed.is_empty() || !outcome.archived.is_empty() || !outcome.restored.is_empty();
							if busy || outcome.no_account {
								info!(
									kept = outcome.kept,
									refused = ?outcome.refused,
									handed = ?outcome.handed,
									archived = ?outcome.archived,
									restored = ?outcome.restored,
									no_account = outcome.no_account,
									"Jobs mailbox pass"
								);
							}
						}
						Err(err) => {
							error!(error = %err, class = err.class(), "Jobs mailbox pass failed");
							crate::metrics::mailbox::record_pass(&err.outcome, Some(err.class()));
						}
					}
				}
			}
		}
	});
}
