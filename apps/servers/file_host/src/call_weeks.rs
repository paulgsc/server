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
//!
//! **What it says.** Each pass's start and whether it failed (CW2), and each
//! minute what the render worker has made of the weeks held ([`render`],
//! CW5): the CALL dashboard and the `CallWeeks*` and `CallRender*` alerts in
//! `infra/prometheus/rules/home.yml`, which reach the owner on Slack.
//!
//! **When the render may run.** [`lane`] decides every five seconds whether
//! home's batch class may run, must pause or must stop (L4b, #708), writes
//! the decision where the render worker reads it, and says it on the CALL
//! dashboard and in the `Lane*` alerts.

pub mod lane;
pub mod pass;
pub mod render;

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

/// Start the worker.
///
/// The render folder is made first, before the server answers its health
/// check, so the render worker never finds it missing.
/// A folder that cannot be made is logged here and counted by every pass.
///
/// So are the render worker's own folders (`render::RENDERS_FOLDER`,
/// `WORK_FOLDER`, `STATUS_FOLDER`): made by this server's uid, which the
/// worker runs as too (`infra/compose/call-render.yml`), so a week the owner
/// lets go from My data is one this server can delete (`feeds::videos::let_go`).
pub fn start(state: &AppState) {
	if std::fs::create_dir_all(RENDER_FOLDER).is_err() {
		error!(error = "render folder", "call weeks worker cannot make its render folder");
	}
	for folder in [render::RENDERS_FOLDER, render::WORK_FOLDER, render::STATUS_FOLDER] {
		if std::fs::create_dir_all(folder).is_err() {
			error!(error = "render folder", "call weeks worker cannot make the render worker's folders");
		}
	}
	spawn(state, state.call_weeks.clone());
	lane::spawn(state);
}

/// Spawn the worker, cancelled through the shared token. Its first pass is
/// due at once, so a restart catches up without waiting an interval.
fn spawn(state: &AppState, context: CallWeeksContext) {
	let cancel = state.core.cancel_token.clone();
	let db = state.core.shared_db.clone();

	tokio::spawn(async move {
		info!(interval_secs = INTERVAL.as_secs(), "call weeks worker started");
		crate::metrics::call_weeks::record_started();
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
					observe_renders(&db).await;
					let now = chrono::Utc::now();
					if !schedule.due(now) {
						continue;
					}
					crate::metrics::call_weeks::record_pass_started();
					let result = pass::run_once(&db, &context.fetch, std::path::Path::new(RENDER_FOLDER), &chrono::Utc::now).await;
					crate::metrics::call_weeks::record_pass(&result);
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

/// What the render worker has made of the weeks held, as metrics. A
/// failing read leaves the last values and logs.
async fn observe_renders(db: &sqlx::SqlitePool) {
	let (renders, status) = (std::path::Path::new(render::RENDERS_FOLDER), std::path::Path::new(render::STATUS_FOLDER));
	if let Ok(seen) = render::observe(db, renders, status).await {
		crate::metrics::call_weeks::record_renders(&seen);
	} else {
		warn!(error = "storage", "call render observation failed");
	}
	let work = std::path::Path::new(render::WORK_FOLDER);
	// The worker's Playoff Delta job keeps its videos, work and cache
	// warnings on the same drive, so the drive's numbers, and
	// CallRenderCacheFull, count both jobs.
	let pd = crate::pd_fills::render::disk().await;
	crate::metrics::call_weeks::record_disk(&render::disk(renders, work, status).await.and(&pd));
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
