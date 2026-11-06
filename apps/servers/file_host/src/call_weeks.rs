//! Home keeps every week of the weekly call (paulgsc/some-ui#2635, ST36's
//! second slice).
//!
//! The weekly routine writes each week's call to `paulgsc/testing`, under
//! `call/<from>.json`, and lists the weeks it keeps in `call/index.json`. That
//! repository keeps only the last few (the index's `keep`), so the owner
//! decided on 2026-11-06 that home keeps every one: this worker reads the
//! index on a timer and stores each week home does not hold yet, in
//! `call_week` (`call_week_repo`). The mailbox pass then lists every week home
//! holds in its `call` index (`mailbox::index::call`), and the routine lets a
//! week leave the repository only once that index lists it.
//!
//! **Why this is a poll.** GitHub tells home nothing, and home makes only
//! outbound requests. A week is written once a week, so the worker reads the
//! index every [`INTERVAL`], an hour, and an idle pass is that one request.
//! The mailbox's note of a new week (MBP, paulgsc/some-ui#2142) can make it
//! immediate later.
//!
//! What this module promises:
//!
//! - **It reads one public folder and writes one table.** `paulgsc/testing`'s
//!   `call/` on `main`, without a token, and `call_week`, append-only. No
//!   person's data is read or written.
//! - **A week is kept once, as first served.** One home holds is never
//!   fetched again, and a later edit in the repository is not taken.
//! - **Logs carry counts and kinds,** never a URL, a body or a week's words.
//! - **It backs off** after a pass that went wrong: [`RETRY`] doubled per
//!   failure in a row, to at most [`INTERVAL`].

pub mod pass;

use crate::AppState;
use std::time::Duration;
use tracing::{error, info, warn};

/// How often home reads the index: a week arrives once a week.
pub const INTERVAL: Duration = Duration::from_secs(3600);

/// How soon a pass that went wrong is tried again, doubled per failure in a
/// row, to at most [`INTERVAL`].
pub const RETRY: Duration = Duration::from_secs(300);

/// How often the worker looks at its schedule. Nothing is fetched unless due.
pub const TICK: Duration = Duration::from_secs(60);

const _: () = assert!(RETRY.as_secs() <= INTERVAL.as_secs() && TICK.as_secs() <= RETRY.as_secs());

/// How long one request may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Everything the worker needs.
///
/// Always on: it reads one public folder and needs no secret, so it is not a
/// variable (`.sops.yaml`, #613's first principle).
#[derive(Clone)]
pub struct CallWeeksContext {
	pub fetch: crate::game_feed::pass::ReqwestFetch,
}

impl CallWeeksContext {
	/// The worker's context once, at startup.
	///
	/// # Errors
	/// An HTTP client that does not build.
	pub fn new() -> anyhow::Result<Self> {
		let client = reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build()?;
		Ok(Self {
			fetch: crate::game_feed::pass::ReqwestFetch::new(client),
		})
	}
}

/// Start the worker.
pub fn start(state: &AppState) {
	spawn(state, state.call_weeks.clone());
}

/// Spawn the worker, cancelled through the shared token. Its first pass is
/// due at once, so a restart catches up without waiting an interval.
fn spawn(state: &AppState, context: CallWeeksContext) {
	let cancel = state.core.cancel_token.clone();
	let db = state.core.shared_db.clone();

	tokio::spawn(async move {
		info!(interval_secs = INTERVAL.as_secs(), "call weeks worker started");
		let mut schedule = pass::Schedule::new(chrono::Utc::now());
		let mut ticker = tokio::time::interval(TICK);
		ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		loop {
			tokio::select! {
				() = cancel.cancelled() => {
					info!("call weeks worker cancelled");
					return;
				}
				_ = ticker.tick() => {
					let now = chrono::Utc::now();
					if !schedule.due(now) {
						continue;
					}
					let result = pass::run_once(&db, &context.fetch, &chrono::Utc::now).await;
					let failed = result.as_ref().map_or(true, |done| !done.failures.is_empty());
					schedule.after(now, failed, INTERVAL, RETRY);
					log_pass(&result);
				}
			}
		}
	});
}

/// The pass's one log line, when it added a week or something went wrong:
/// counts and kinds, never a URL, a body or a week's words.
fn log_pass(result: &Result<pass::Pass, sqlx::Error>) {
	match result {
		Ok(done) if done.added == 0 && done.failures.is_empty() => {}
		Ok(done) if done.failures.is_empty() => info!(added = done.added, held = done.held, "call weeks pass"),
		Ok(done) => warn!(added = done.added, held = done.held, failures = ?done.failures, "call weeks pass"),
		Err(_) => error!(error = "storage", "call weeks pass failed"),
	}
}
