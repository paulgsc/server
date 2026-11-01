//! Home follows the games of the week (paulgsc/some-ui#2076, GWB2).
//!
//! The owner records voice bites about NFL games; later stories match each
//! bite to the play it is about (GWB7) and offer what can be happening now
//! (GWB3). Both read the play log this worker keeps from ESPN's unofficial
//! site API (`nfl_feed`), which the owner chose on 2026-10-25.
//!
//! **Why this is a poll.** ESPN pushes nothing to home, and home makes only
//! outbound requests. The worker wakes every [`TICK`], fetches only what its
//! [`pass::Schedule`] says is due, and an idle wake does no I/O at all: the
//! scoreboard every 10 minutes ([`CADENCE`]), a game's summary
//! every 15 seconds while it is live, once more after it
//! goes final. GW6 asks for a play within a minute, 8 times in 10; the live
//! cadence plus one tick is 20 s, which leaves the rest to ESPN's own delay.
//!
//! What this module promises (`docs/game-feed.md` in this app):
//!
//! - **Every response is kept, raw, before it is read** (GW10), in
//!   `game_feed_response`, its bytes once by SHA-256 in `game_feed_body`,
//!   and the play log (`game_feed_play`) is appended from it in the same
//!   transaction. All three tables are append-only; nothing here updates or
//!   deletes, and a revised play is a new row.
//! - **It reads ESPN's public API and writes only those three tables.** No
//!   route serves them yet, and no person's data is read or written.
//! - **Logs and metrics carry counts and kinds,** never a URL, a body or a
//!   play's text.
//! - **It backs off** on a 429, any other refusal, an unreadable body or no
//!   answer, exponentially and to at most 15 minutes.

pub mod pass;

use crate::{AppState, Config};
use std::time::Duration;
use tracing::{error, info};

/// How often the worker looks at its schedule. Nothing is fetched unless due.
pub const TICK: Duration = Duration::from_secs(5);

/// The shortest scoreboard cadence.
pub const MIN_SCOREBOARD: Duration = Duration::from_secs(60);

/// The shortest live cadence: ESPN caches its responses for 5 s
/// (`cache-control: max-age=5`), so asking more often gets the same bytes.
pub const MIN_LIVE: Duration = Duration::from_secs(5);
/// The longest any cadence or the backoff cap may be: a day. Past it a
/// fetch would be due beyond any game it is for, and a time that far out can
/// overflow the scheduler's arithmetic.
pub const MAX_CADENCE: Duration = Duration::from_secs(86_400);

/// The feed's cadences: source constants, not variables (#617), each its
/// variable's default (`GAME_FEED_SCOREBOARD_SECONDS`,
/// `GAME_FEED_LIVE_SECONDS`, `GAME_FEED_BACKOFF_CAP_SECONDS`). The backoff
/// cap is at least both cadences and every value is within the floors and
/// [`MAX_CADENCE`]: the assertions below hold that at compile time.
pub const CADENCE: pass::Cadence = pass::Cadence {
	scoreboard: Duration::from_secs(600),
	live: Duration::from_secs(15),
	backoff_cap: Duration::from_secs(900),
};

const _: () = {
	let (scoreboard, live, cap) = (CADENCE.scoreboard.as_secs(), CADENCE.live.as_secs(), CADENCE.backoff_cap.as_secs());
	assert!(scoreboard >= MIN_SCOREBOARD.as_secs() && live >= MIN_LIVE.as_secs());
	assert!(cap <= MAX_CADENCE.as_secs() && cap >= scoreboard && cap >= live);
};

/// How long one request may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Everything the worker needs, or `None` when the deployment has not turned
/// the feed on.
#[derive(Clone)]
pub struct GameFeedContext {
	pub fetch: pass::ReqwestFetch,
	pub cadence: pass::Cadence,
}

impl GameFeedContext {
	/// The feed's context once, at startup. Off is fine and logged.
	///
	/// # Errors
	/// `GAME_FEED_ENABLED` with an HTTP client that does not build.
	pub fn from_config(config: &Config) -> anyhow::Result<Option<Self>> {
		if !config.game_feed_enabled {
			info!("the game feed is not enabled; home follows no NFL game");
			return Ok(None);
		}
		let cadence = CADENCE;
		let client = reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build()?;
		Ok(Some(Self {
			fetch: pass::ReqwestFetch::new(client),
			cadence,
		}))
	}
}

/// Start the worker if the deployment turned it on, and say which on
/// `game_feed_enabled` either way.
pub fn start(state: &AppState) {
	crate::metrics::game_feed::record_enabled(state.game_feed.is_some());
	if let Some(context) = state.game_feed.clone() {
		spawn(state, context);
	}
}

/// Spawn the worker, cancelled through the shared token. The scoreboard is
/// due at once, so a restart catches up without waiting a cadence.
fn spawn(state: &AppState, context: GameFeedContext) {
	let cancel = state.core.cancel_token.clone();
	let db = state.core.shared_db.clone();
	crate::metrics::game_feed::record_started();

	tokio::spawn(async move {
		info!(
			scoreboard_secs = context.cadence.scoreboard.as_secs(),
			live_secs = context.cadence.live.as_secs(),
			"game feed started"
		);
		let mut schedule = pass::Schedule::new(chrono::Utc::now());
		let mut ticker = tokio::time::interval(TICK);
		ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		loop {
			tokio::select! {
				() = cancel.cancelled() => {
					info!("game feed cancelled");
					return;
				}
				_ = ticker.tick() => {
					crate::metrics::game_feed::record_pass_started();
					let result = pass::run_once(&db, &context.fetch, &mut schedule, context.cadence, &chrono::Utc::now).await;
					crate::metrics::game_feed::record_pass(result.as_ref().ok(), schedule.followed(chrono::Utc::now()));
					log_pass(&result);
				}
			}
		}
	});
}

/// The pass's one log line, when it fetched anything: the kind of each
/// fetch, and how many plays it added. Never a URL, a body or a play.
pub(crate) fn log_pass(result: &Result<pass::Pass, sqlx::Error>) {
	match result {
		Ok(done) if done.fetched.is_empty() => {}
		Ok(done) => info!(fetched = ?done.fetched, added = done.added, "game feed pass"),
		Err(_) => error!(error = "storage", "game feed pass failed"),
	}
}
