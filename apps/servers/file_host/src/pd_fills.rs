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
//! It also writes each voiced fill it holds to [`render::EPISODES_FOLDER`]
//! for home's render worker, which records and renders the week from it
//! (`infra/compose/call-render.yml`).
//!
//! **What it says.** Each pass's start and whether it failed (PK2), and each
//! minute what the render worker has made of the fills held ([`render`],
//! PK5): the CALL dashboard and the `PlayoffDelta*` alerts in
//! `infra/prometheus/rules/home.yml`, which reach the owner on Slack.

pub mod pass;
pub mod render;

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
///
/// Its folders and the render worker's are made first, before the server
/// answers its health check, by this server's uid, which the worker runs as
/// too, so the worker never finds one missing or not its own.
pub fn start(state: &AppState) {
	for folder in [render::EPISODES_FOLDER, render::RENDERS_FOLDER, render::WORK_FOLDER, render::STATUS_FOLDER] {
		if std::fs::create_dir_all(folder).is_err() {
			error!(error = "render folder", "playoff delta fills worker cannot make the render worker's folders");
		}
	}
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
					observe_renders(&db, std::path::Path::new(render::RENDERS_FOLDER), std::path::Path::new(render::STATUS_FOLDER)).await;
					let now = chrono::Utc::now();
					if !schedule.due(now) {
						continue;
					}
					crate::metrics::pd_fills::record_pass_started();
					let result = run_once(&db, &fetch).await;
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

/// One pass ([`pass::run_once`]), then the render worker's folder written
/// ([`render::export`]), whatever the pass fetched: home's fills are
/// written for the worker even when the repository can't be read.
async fn run_once(db: &sqlx::SqlitePool, fetch: &crate::game_feed::pass::ReqwestFetch) -> Result<pass::Pass, sqlx::Error> {
	let mut done = pass::run_once(db, fetch, &chrono::Utc::now).await?;
	let exported = render::export(db, std::path::Path::new(render::EPISODES_FOLDER)).await?;
	done.exported = exported.written;
	done.failures.extend(std::iter::repeat_n("export", exported.failed));
	Ok(done)
}

/// What the render worker has made of the fills held, as metrics. A failing
/// read leaves the last values and logs, and is held open until a tick reads
/// it (#1179).
async fn observe_renders(db: &sqlx::SqlitePool, renders: &std::path::Path, status: &std::path::Path) {
	if let Ok(seen) = render::observe(db, renders, status).await {
		crate::metrics::pd_fills::record_renders(&seen);
		crate::failure::open(crate::failure::Step::PdFillsObserve, 0);
	} else {
		warn!(error = "storage", "playoff delta render observation failed");
		crate::failure::open(crate::failure::Step::PdFillsObserve, 1);
	}
}

/// The pass's one log line, when it added a fill or something went wrong.
fn log_pass(result: &Result<pass::Pass, sqlx::Error>) {
	match result {
		Ok(done) if done.added == 0 && done.exported == 0 && done.failures.is_empty() => {}
		Ok(done) if done.failures.is_empty() => info!(added = done.added, held = done.held, exported = done.exported, "playoff delta fills pass"),
		Ok(done) => warn!(added = done.added, held = done.held, exported = done.exported, failures = ?done.failures, "playoff delta fills pass"),
		Err(_) => error!(error = "storage", "playoff delta fills pass failed"),
	}
}

#[cfg(test)]
mod tests {
	use super::observe_renders;
	use crate::failure::{held, Step};
	use metrics_util::debugging::DebuggingRecorder;
	use sqlx::sqlite::SqlitePoolOptions;

	/// #1179: a tick that cannot read what the worker made of the fills holds
	/// it open, and a tick that reads it clears it.
	#[tokio::test]
	async fn an_observation_not_read_is_held_open_until_one_is() {
		let recorder = DebuggingRecorder::new();
		let snapshotter = recorder.snapshotter();
		let _recording = metrics::set_default_local_recorder(&recorder);
		let dir = tempfile::tempdir().unwrap();
		let (renders, status) = (dir.path().join("pd"), dir.path().join("pd-worker"));

		let bare = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		crate::failure::open(Step::PdFillsObserve, 9);
		observe_renders(&bare, &renders, &status).await;
		assert_eq!(held::open(&snapshotter, Step::PdFillsObserve), Some(1.0));

		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		crate::schema::MIGRATOR.run(&pool).await.unwrap();
		crate::failure::open(Step::PdFillsObserve, 9);
		observe_renders(&pool, &renders, &status).await;
		assert_eq!(held::open(&snapshotter, Step::PdFillsObserve), Some(0.0));
	}
}
