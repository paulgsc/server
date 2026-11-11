//! Home keeps each clip's frames (paulgsc/some-ui#2670's clips; the owner's
//! decisions of 2026-11-11, call C125).
//!
//! A clip is what a user-facing change built, as a user sees it: a `before`
//! and an `after`, each a few stills with a label (`@some-ui/video-render`'s
//! `clip.ts`). `paulgsc/testing` lists them in `clips/index.json` and keeps no
//! media (testing#6), so home keeps the frames, by how each side was made:
//!
//! - **A walk side** (`made: "walk"`, with `walk: {app, sample}` from
//!   `some-ui.clip-index/2`): home makes its frames itself, on its render
//!   lane. This worker fetches the walk's sample (named from the clip's own
//!   folder: `after/sample.json` is `clips/<clip>/after/sample.json`) and
//!   writes it, with the app's name, to [`WALKS_FOLDER`]; the render worker
//!   runs `motion walk` on it when the lane admits it
//!   (`infra/render/render-calls.sh`'s `clip-walk` job) into [`WALK_RENDERS_FOLDER`]; the next pass checks that the walk's
//!   `walk.json` names the index's snips, in order, by the same files and
//!   labels, and only then keeps them.
//! - **The six legacy frames** (`made: "session"`, named in [`pass::LEGACY`]):
//!   fetched from the repository and kept only if each is byte for byte the
//!   one pinned there, copied from testing's `scripts/guard.py`.
//! - **Any other session-taken side:** nothing; its labels are the index's.
//!
//! What is kept goes under [`FRAMES_FOLDER`], `<clip>/<side>/<file>`, beside
//! the other renders, with a row in `clip_frame` (`clip_frame_repo`). It is
//! append-only: a held frame is never replaced, and other bytes under a held
//! name are a failure, never written. Each frame is a PNG by its magic bytes,
//! at most [`pass::MAX_FRAME`], and a walk side has at most
//! [`pass::MAX_SNIPS`]: well under 50 MB a year.
//!
//! **The walk's honest limit.** A walk is drawn by the render image's own
//! pinned some-ui, not at the commit of the change the clip shows: the render
//! network reaches nothing, so it cannot fetch another. What guards it is the
//! check above: a walk whose snips or labels differ from the index's is
//! `mismatch`, and nothing of it is kept. Such a walk is stuck until the
//! image moves to a some-ui that draws it, and is seen two ways: the pass's
//! log line counts `mismatch` every hour, and a walk the worker could not run
//! at all is on the phone's Renders page as failed, its whole log on home in
//! `renders/clip-walk-worker/<clip>-<side>.failed.log`.
//!
//! **Why this is a poll,** as `call_weeks`: GitHub tells home nothing, and a
//! clip lands a few times a week. An idle pass is one request for the index.
//!
//! **What it says.** Each pass's counts and failures, by kind, in its one log
//! line; a walk that couldn't render is on the phone's Renders page, and its
//! log on home (`<status>/<key>.failed.log`). No URL, body or label reaches a
//! log.

pub mod pass;

use crate::AppState;
use std::path::Path;
use std::time::Duration;
use tracing::{error, info, warn};

/// How often home reads the index.
pub const INTERVAL: Duration = Duration::from_secs(3600);

/// A pass that went wrong is tried again after this, doubled per failure in
/// a row, to at most [`INTERVAL`].
pub const RETRY: Duration = Duration::from_secs(300);

/// How often the worker looks at its schedule. Nothing is fetched unless due:
/// a walk the render worker finished is taken by the next pass, within the
/// hour.
pub const TICK: Duration = Duration::from_secs(60);

const _: () = assert!(RETRY.as_secs() <= INTERVAL.as_secs() && TICK.as_secs() <= RETRY.as_secs());

/// Where the frames home keeps are: `<clip>/<side>/<file>`, beside the
/// call's, the essays' and the promos' renders, on the same drive.
pub const FRAMES_FOLDER: &str = "/app/dbs/renders/clips";

/// Where each walk home is to render is written for the render worker.
///
/// One folder a side, `<clip>-<side>`, holding `sample.json` and `app`.
/// `infra/compose/call-render.yml` mounts it read-only at `/clip-walks`.
pub const WALKS_FOLDER: &str = "/app/dbs/clip-walks";

/// Where the render worker leaves each finished walk, whole: `<clip>-<side>/`
/// holding `walk.json` and its `snips/`.
pub const WALK_RENDERS_FOLDER: &str = "/app/dbs/renders/clip-walk";

/// The render worker's status folder for walks: `last-pass`, `took`, and a
/// failed walk's `<key>.failed.log`.
pub const WALK_STATUS_FOLDER: &str = "/app/dbs/renders/clip-walk-worker";

/// The render worker's work folder for walks.
pub const WALK_WORK_FOLDER: &str = "/app/dbs/renders/clip-walk-work";

/// The file a finished walk is known by, as a video is for the other jobs.
pub const WALK_FILE: &str = "walk.json";

/// Start the worker.
///
/// Its folders are made first, as this server's uid, before the server
/// answers its health check, so the render worker never finds one missing
/// and never makes one Docker's.
pub fn start(state: &AppState) {
	for folder in [FRAMES_FOLDER, WALKS_FOLDER, WALK_RENDERS_FOLDER, WALK_STATUS_FOLDER, WALK_WORK_FOLDER] {
		if std::fs::create_dir_all(folder).is_err() {
			error!(error = "folder", "clips worker cannot make its folders");
		}
	}
	let cancel = state.core.cancel_token.clone();
	let db = state.core.shared_db.clone();
	let fetch = state.call_weeks.fetch.clone();

	tokio::spawn(async move {
		info!(interval_secs = INTERVAL.as_secs(), "clips worker started");
		let folders = pass::Folders {
			frames: Path::new(FRAMES_FOLDER).to_path_buf(),
			walks: Path::new(WALKS_FOLDER).to_path_buf(),
			walk_renders: Path::new(WALK_RENDERS_FOLDER).to_path_buf(),
		};
		let mut schedule = crate::game_feed::pass::Target::now(chrono::Utc::now());
		let mut ticker = tokio::time::interval(TICK);
		ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		loop {
			tokio::select! {
				() = cancel.cancelled() => {
					info!("clips worker cancelled");
					return;
				}
				_ = ticker.tick() => {
					let now = chrono::Utc::now();
					if !schedule.due(now) {
						continue;
					}
					let result = pass::run_once(&db, &fetch, &folders, &chrono::Utc::now).await;
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

/// The pass's one log line, when it did something or something went wrong.
fn log_pass(result: &Result<pass::Pass, sqlx::Error>) {
	match result {
		Ok(done) if done.kept == 0 && done.queued == 0 && done.failures.is_empty() => {}
		Ok(done) if done.failures.is_empty() => info!(kept = done.kept, held = done.held, queued = done.queued, "clips pass"),
		Ok(done) => warn!(kept = done.kept, held = done.held, queued = done.queued, failures = ?done.failures, "clips pass"),
		Err(_) => error!(error = "storage", "clips pass failed"),
	}
}
