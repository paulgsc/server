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
//! It reads one public folder, `paulgsc/testing`'s `call/` on `main`,
//! without a token, and writes one table, `call_week`, append-only; a week is
//! kept once, as first served ([`pass`]'s module doc has the rest). It also
//! writes each week it holds to [`RENDER_FOLDER`] for home's render worker,
//! which records and renders the call from it (`infra/compose/call-render.yml`).

pub mod pass;

use crate::AppState;
use std::time::Duration;
use tracing::{error, info, warn};

/// How often home reads the index: a week arrives once a week.
pub const INTERVAL: Duration = Duration::from_secs(3600);

/// A pass that went wrong is tried again after this doubled per failure in a
/// row (ten minutes, then twenty, ...), to at most [`INTERVAL`].
pub const RETRY: Duration = Duration::from_secs(300);

/// Where each week home holds is written for the render worker.
///
/// Each week is `<from>.json`, byte for byte, in `call-weeks/` beside the
/// database: on the host, `${STORAGE}/call-weeks`, on the same drive as
/// everything else home keeps. `infra/compose/call-render.yml` mounts it
/// read-only into the worker, which starts only once this server is healthy,
/// so the folder is always made here, by this server's uid ([`start`]).
pub const RENDER_FOLDER: &str = "/app/dbs/call-weeks";

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

/// Start the worker. The render folder is made first, before the server
/// answers its health check, so the render worker never finds it missing.
/// A folder that cannot be made is logged here and counted by every pass.
pub fn start(state: &AppState) {
	if std::fs::create_dir_all(RENDER_FOLDER).is_err() {
		error!(error = "render folder", "call weeks worker cannot make its render folder");
	}
	spawn(state, state.call_weeks.clone());
}

/// Spawn the worker, cancelled through the shared token. Its first pass is
/// due at once, so a restart catches up without waiting an interval.
fn spawn(state: &AppState, context: CallWeeksContext) {
	let cancel = state.core.cancel_token.clone();
	let db = state.core.shared_db.clone();

	tokio::spawn(async move {
		info!(interval_secs = INTERVAL.as_secs(), "call weeks worker started");
		let mut schedule = crate::game_feed::pass::Target::now(chrono::Utc::now());
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
					let result = pass::run_once(&db, &context.fetch, std::path::Path::new(RENDER_FOLDER), &chrono::Utc::now).await;
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

/// The pass's one log line, when it added a week or something went wrong.
fn log_pass(result: &Result<pass::Pass, sqlx::Error>) {
	match result {
		Ok(done) if done.added == 0 && done.exported == 0 && done.failures.is_empty() => {}
		Ok(done) if done.failures.is_empty() => info!(added = done.added, held = done.held, exported = done.exported, "call weeks pass"),
		Ok(done) => warn!(added = done.added, held = done.held, exported = done.exported, failures = ?done.failures, "call weeks pass"),
		Err(_) => error!(error = "storage", "call weeks pass failed"),
	}
}
