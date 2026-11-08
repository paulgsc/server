//! Home keeps every fill of Playoff Delta (paulgsc/some-ui#2647, FEED4).
//!
//! The narration routine writes each week's fill (what is said over each
//! scene, `some-ui.pd-narration/1`) to `paulgsc/testing`, under
//! `playoff-delta/<season>/narration/`, and lists every fill there in
//! `playoff-delta/index.json`. This worker reads that index on a timer and
//! keeps each fill home does not hold yet, with its season file as served
//! then, in `pd_fill` (`pd_fill_repo`), so home can make the week's video
//! from what it holds alone.
//!
//! **Why this is a poll,** as `call_weeks`' is: GitHub tells home nothing,
//! and home makes only outbound requests. A fill is written once a week, so
//! the worker reads the index every [`INTERVAL`], an hour, and an idle pass is
//! that one request.
//!
//! It reads one public folder, `paulgsc/testing`'s `playoff-delta/` on
//! `main`, without a token, and writes one table, `pd_fill`, append-only; a
//! fill is kept once, as first served ([`pass`]'s module doc has the rest).
//!
//! **What it says.** Each pass's start and whether it failed (PK2): the
//! `PlayoffDeltaFills*` alerts in `infra/prometheus/rules/home.yml`, which
//! reach the owner on Slack.

pub mod pass;

use crate::AppState;
use std::time::Duration;
use tracing::{error, info, warn};

/// How often home reads the index: a fill arrives once a week.
pub const INTERVAL: Duration = Duration::from_secs(3600);

/// A pass that went wrong is tried again after this doubled per failure in a
/// row (ten minutes, then twenty, ...), to at most [`INTERVAL`].
pub const RETRY: Duration = Duration::from_secs(300);

/// How often the worker looks at its schedule. Nothing is fetched unless due.
pub const TICK: Duration = Duration::from_secs(60);

const _: () = assert!(RETRY.as_secs() <= INTERVAL.as_secs() && TICK.as_secs() <= RETRY.as_secs());

/// Start the worker.
///
/// Always on, as the call's: it reads one public folder and needs no secret.
/// It shares the call weeks worker's HTTP client
/// (`call_weeks::CallWeeksContext`), which reads the same repository.
pub fn start(state: &AppState) {
	let cancel = state.core.cancel_token.clone();
	let db = state.core.shared_db.clone();
	let fetch = state.call_weeks.fetch.clone();

	tokio::spawn(async move {
		info!(interval_secs = INTERVAL.as_secs(), "playoff delta fills worker started");
		crate::metrics::pd_fills::record_started();
		let mut schedule = crate::game_feed::pass::Target::now(chrono::Utc::now());
		let mut ticker = tokio::time::interval(TICK);
		ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		loop {
			tokio::select! {
				() = cancel.cancelled() => {
					info!("playoff delta fills worker cancelled");
					return;
				}
				_ = ticker.tick() => {
					let now = chrono::Utc::now();
					if !schedule.due(now) {
						continue;
					}
					crate::metrics::pd_fills::record_pass_started();
					let result = pass::run_once(&db, &fetch, &chrono::Utc::now).await;
					crate::metrics::pd_fills::record_pass(&result);
					if result.as_ref().is_ok_and(|done| done.failures.is_empty()) {
						schedule.succeeded(now, INTERVAL);
					} else {
						schedule.failed(now, RETRY, INTERVAL);
					}
					log_pass(&result);
				}
			}
		}
	});
}

/// The pass's one log line, when it added a fill or something went wrong.
fn log_pass(result: &Result<pass::Pass, sqlx::Error>) {
	match result {
		Ok(done) if done.added == 0 && done.failures.is_empty() => {}
		Ok(done) if done.failures.is_empty() => info!(added = done.added, held = done.held, "playoff delta fills pass"),
		Ok(done) => warn!(added = done.added, held = done.held, failures = ?done.failures, "playoff delta fills pass"),
		Err(_) => error!(error = "storage", "playoff delta fills pass failed"),
	}
}
