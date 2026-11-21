//! Home keeps every cards essay and gives it to the render worker (CARDS7,
//! #854, paulgsc/some-ui#2979; call C327).
//!
//! A session writes a cards essay, a video's cut list (spoken since the
//! cheat-sheet series' v2: each scene said by home's speaker, its chapter's
//! intro included, EXPLAIN6 #878) whose every card is stamped with the
//! commits it was checked against, to `paulgsc/testing` as
//! `cards/<key>.json`, and lists it in `cards/index.json`
//! (paulgsc/some-ui `packages/video-render/CARDS-ROUTINE.md`). This worker
//! reads that index on a timer and stores each essay home does not hold yet
//! in `cards_essay` (`cards_essay_repo`), then writes it to
//! [`ESSAYS_FOLDER`] as `<key>.json` for home's render worker, whose
//! `cards-essay` job records the stage's burn-ins page and renders it
//! (`infra/render/render-calls.sh`). The owner is its audience; nothing about
//! it is uploaded anywhere.
//!
//! **Why this is a poll,** as the call's weeks are (`call_weeks`): GitHub
//! tells home nothing, and home makes only outbound requests. An essay comes
//! when a session writes one, so the worker reads the index every
//! [`INTERVAL`], an hour, and an idle pass is that one request.
//!
//! It reads one public folder, `paulgsc/testing`'s `cards/` on `main`,
//! without a token, and writes one table, `cards_essay`, append-only: an
//! essay is kept once, as first served ([`pass`]'s module doc has the rest).
//!
//! **What it says.** Each pass's start and whether it failed, and each
//! minute what the render worker has made of the essays held ([`observe`]):
//! the `CardsEssay*` alerts in `infra/prometheus/rules/home.yml`, which reach
//! the owner on Slack. A failed render is the worker's `<key>.failed.log`, as
//! every job's.
//!
//! **Where the owner watches it** (C338): Studio › Renders lists each essay
//! (the `cards-essay` shelf, `feeds::renders`), and a paired phone plays a
//! done one from `GET /cards-essays/:key/video` (`routes::cards_essays`).
//! Its `YouTube` link is recorded as every feed's
//! (`PUT /feeds/cards-essay/videos/:key/link`, `cards_essay_link`;
//! paulgsc/some-ui#2999, C388). Nothing lets home's copy go yet: the owner
//! ruled that a cards essay lives like every other episode, its copy let go
//! once a confirmed link is recorded (`feeds::videos`), which is a later
//! story.
//!
//! **A spoken card's takes** (WALK7, paulgsc/server#1052): an essay with a
//! spoken card is given with the owner's takes it names, held back until
//! each is heard, or refused (a failed row on Studio › Renders) when one is
//! missing or home couldn't hear it ([`pass`]'s module doc has the rest).
//!
//! Cost: one request an hour, plus one per essay home lacks (a cut list is a
//! few KB); per essay not yet given with a spoken card, a few indexed reads
//! per take it names, each pass until it is given; one `stat` per essay
//! each minute.

pub mod pass;

use crate::call_weeks::render::{tally, Held, Renders};
use crate::AppState;
use sqlx::SqlitePool;
use std::path::Path;
use std::time::Duration;
use tracing::{error, info, warn};

/// How often home reads the index: an essay comes when a session writes one.
pub const INTERVAL: Duration = Duration::from_secs(3600);

/// A pass that went wrong is tried again after this doubled per failure in a
/// row, to at most [`INTERVAL`], as the call's weeks are.
pub const RETRY: Duration = Duration::from_secs(300);

/// How often the worker looks at its schedule and at what the render worker
/// made. Nothing is fetched unless due.
pub const TICK: Duration = Duration::from_secs(60);

const _: () = assert!(RETRY.as_secs() <= INTERVAL.as_secs() && TICK.as_secs() <= RETRY.as_secs());

/// How long one request may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Where each essay home holds is written for the render worker.
///
/// Each is `<key>.json`, byte for byte: on the host,
/// `${STORAGE}/cards-essays`, mounted read-only into the worker as
/// `/cards-essays` (`infra/compose/call-render.yml`).
pub const ESSAYS_FOLDER: &str = "/app/dbs/cards-essays";

/// Where the worker keeps each essay's video, as this server sees it.
pub const RENDERS_FOLDER: &str = "/app/dbs/renders/cards-essay";

/// Where the worker says how its passes went, as this server sees it.
pub const STATUS_FOLDER: &str = "/app/dbs/renders/cards-essay-worker";

/// Where the worker keeps a waiting essay's work and step cache between
/// attempts, as this server sees it.
pub const WORK_FOLDER: &str = "/app/dbs/renders/cards-essay-work";

/// The video's file in an essay's folder (`record:cards`' `cards.mp4`).
pub const VIDEO: &str = "cards.mp4";

/// Everything the worker needs.
///
/// Always on: it reads one public folder and needs no secret, so it is not a
/// variable (`.sops.yaml`, #613's first principle).
#[derive(Clone)]
pub struct CardsEssaysContext {
	pub fetch: crate::game_feed::pass::ReqwestFetch,
}

impl CardsEssaysContext {
	/// The worker's context once, at startup.
	///
	/// # Errors
	/// An HTTP client that does not build.
	pub fn new() -> anyhow::Result<Self> {
		let client = outbound::client(outbound::Caller::CardsEssays).timeout(REQUEST_TIMEOUT).build()?;
		Ok(Self {
			fetch: crate::game_feed::pass::ReqwestFetch::new(client),
		})
	}
}

/// Start the worker.
///
/// Its folder and the render worker's are made first, before the server
/// answers its health check, by this server's uid, which the worker runs as
/// too (`infra/compose/call-render.yml`), so the worker never finds one
/// missing or not its own. A folder that cannot be made is logged here and
/// counted by every pass.
pub fn start(state: &AppState) {
	for folder in [ESSAYS_FOLDER, RENDERS_FOLDER, WORK_FOLDER, STATUS_FOLDER] {
		if std::fs::create_dir_all(folder).is_err() {
			error!(error = "render folder", "cards essays worker cannot make the render worker's folders");
		}
	}
	let context = state.cards_essays.clone();
	let configured = state.mailbox.as_ref().and_then(|mailbox| mailbox.subject.clone());
	// The mailbox pass fetches a waiting essay's snips when the deployment
	// gave it a token (`snips`); without one, a snip is missing.
	let fetching = state.mailbox.as_ref().is_some_and(|mailbox| mailbox.snips.is_some());
	let cancel = state.core.cancel_token.clone();
	let db = state.core.shared_db.clone();

	// Its first pass is due at once, so a restart catches up without waiting
	// an interval.
	tokio::spawn(async move {
		info!(interval_secs = INTERVAL.as_secs(), "cards essays worker started");
		crate::metrics::cards_essays::record_started();
		let mut schedule = crate::game_feed::pass::Target::now(chrono::Utc::now());
		let mut ticker = tokio::time::interval(TICK);
		ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		loop {
			tokio::select! {
				() = cancel.cancelled() => {
					info!("cards essays worker cancelled");
					return;
				}
				_ = ticker.tick() => {
					observe_renders(&db).await;
					tidy_tick(&db, configured.as_deref(), fetching).await;
					let now = chrono::Utc::now();
					if !schedule.due(now) {
						continue;
					}
					crate::metrics::cards_essays::record_pass_started();
					let result = match given(&db, configured.as_deref(), fetching).await {
						Ok(given) => pass::run_once(&db, &context.fetch, &given, &chrono::Utc::now).await,
						Err(err) => Err(err),
					};
					crate::metrics::cards_essays::record_pass(&result);
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

/// Where this pass gives its essays, and whose takes a spoken card plays:
/// the mailbox's subject (`mailbox::pass_subject`), as `rust_essays`' takes
/// are, or none.
async fn given(db: &SqlitePool, configured: Option<&str>, fetching: bool) -> Result<pass::Given, sqlx::Error> {
	let subject = match crate::mailbox::pass_subject(db, configured).await? {
		crate::auth::owner::Owner::Subject(subject) => Some(subject),
		_ => None,
	};
	Ok(pass::Given {
		essays: std::path::PathBuf::from(ESSAYS_FOLDER),
		status: std::path::PathBuf::from(STATUS_FOLDER),
		subject,
		renders: std::path::PathBuf::from(RENDERS_FOLDER),
		frames: std::path::PathBuf::from(crate::clips::FRAMES_FOLDER),
		snips: fetching.then(|| std::path::PathBuf::from(crate::snips::FOLDER)),
	})
}

/// Each minute, [`pass::tidy`]: no copy of a take outlives its need. Logged
/// only when it did something, by counts.
async fn tidy_tick(db: &SqlitePool, configured: Option<&str>, fetching: bool) {
	let tidied = match given(db, configured, fetching).await {
		Ok(given) => pass::tidy(db, &given).await,
		Err(err) => Err(err),
	};
	match tidied {
		Ok(done) if done == pass::Tidied::default() => {}
		Ok(done) => info!(takes_removed = done.takes_removed, withdrawn = done.withdrawn, failed = done.failed, "cards essays tidied"),
		Err(_) => error!(error = "storage", "cards essays tidy failed"),
	}
}

/// What the render worker has made of the essays home holds.
///
/// Each is rendered (`<renders>/<key>/cards.mp4`) or waiting, and failed
/// when `<status>/<key>.failed.log` is there, as every job's
/// (`call_weeks::render::tally`). Nothing is let go yet (some-ui#2999).
///
/// # Errors
/// A storage failure reading `cards_essay`.
pub async fn observe(db: &SqlitePool, renders: &Path, status: &Path) -> Result<Renders, sqlx::Error> {
	let held = held(db).await?;
	Ok(tally(&held, &[], renders, status, VIDEO).await)
}

/// Each essay home holds, with when it took it, oldest day first.
///
/// # Errors
/// A storage failure reading `cards_essay`.
pub(crate) async fn held(db: &SqlitePool) -> Result<Vec<Held>, sqlx::Error> {
	let mut conn = db.acquire().await?;
	Ok(
		cards_essay_repo::held(&mut conn)
			.await?
			.into_iter()
			.map(|essay| Held {
				key: essay.key,
				taken_at: essay.taken_at,
			})
			.collect(),
	)
}

/// What the cards essays' job takes of the drive (`call_weeks::render::disk`),
/// which the call's observation adds to its own, so the drive's numbers and
/// `CallRenderCacheFull` count every job.
pub async fn disk() -> crate::call_weeks::render::Disk {
	crate::call_weeks::render::disk(Path::new(RENDERS_FOLDER), Path::new(WORK_FOLDER), Path::new(STATUS_FOLDER)).await
}

/// What the render worker has made, as metrics. A failing read leaves the
/// last values and logs.
async fn observe_renders(db: &SqlitePool) {
	if let Ok(seen) = observe(db, Path::new(RENDERS_FOLDER), Path::new(STATUS_FOLDER)).await {
		crate::metrics::cards_essays::record_renders(&seen);
	} else {
		warn!(error = "storage", "cards essay render observation failed");
	}
}

/// The pass's one log line, when it added an essay or something went wrong.
/// It counts; it never names an essay's words.
fn log_pass(result: &Result<pass::Pass, sqlx::Error>) {
	match result {
		Ok(done) if done.added == 0 && done.exported == 0 && done.refused == 0 && done.failures.is_empty() => {}
		Ok(done) if done.failures.is_empty() => info!(
			added = done.added,
			held = done.held,
			exported = done.exported,
			waiting = done.waiting,
			refused = done.refused,
			"cards essays pass"
		),
		Ok(done) => warn!(
			added = done.added,
			held = done.held,
			exported = done.exported,
			waiting = done.waiting,
			refused = done.refused,
			failures = ?done.failures,
			"cards essays pass"
		),
		Err(_) => error!(error = "storage", "cards essays pass failed"),
	}
}
