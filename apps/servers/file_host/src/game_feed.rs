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
//!   route serves them; the paired phone's live game day
//!   (`websocket::game_live`, GWB4) reads them to make its offers, and is
//!   told after each pass that kept something. No person's data is read or
//!   written.
//! - **Logs and metrics carry counts and kinds,** never a URL, a body or a
//!   play's text.
//! - **It backs off** on a 429, any other refusal, an unreadable body or no
//!   answer, exponentially and to at most 15 minutes.
//!
//! **Always started; the operator pauses it.** Whether home follows the
//! games is a lever, `settings::GameFeed` (on by default), set from the web
//! app (`PUT /api/v1/admin/settings/game-feed`), not an environment
//! variable: `GAME_FEED_ENABLED` is gone (owner, 2026-11-13). Each pass reads
//! it once, first, so an idle wake's only I/O is that one indexed read.
//! Paused, the pass fetches nothing and keeps what it has: no row is touched
//! and the schedule waits, so the first pass after it is turned back on
//! fetches whatever is due by then, the scoreboard included. Pausing and
//! resuming are logged once, when the worker first sees them; a switch it
//! cannot read is an error, and that pass is skipped rather than run on a
//! guess.

pub mod pass;

use crate::settings::{self, GameFeed};
use crate::AppState;
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

/// The feed's cadences.
///
/// Source constants, not variables (#617), each its variable's default (`GAME_FEED_SCOREBOARD_SECONDS`,
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

/// Everything the worker needs.
#[derive(Clone)]
pub struct GameFeedContext {
	pub fetch: pass::ReqwestFetch,
	pub cadence: pass::Cadence,
}

impl GameFeedContext {
	/// The feed's context once, at startup.
	///
	/// # Errors
	/// An HTTP client that does not build.
	pub fn new() -> anyhow::Result<Self> {
		let client = reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build()?;
		Ok(Self {
			fetch: pass::ReqwestFetch::new(client),
			cadence: CADENCE,
		})
	}
}

/// Start the worker. It always runs; the operator's switch pauses its passes.
pub fn start(state: &AppState) {
	spawn(state, state.game_feed.clone());
}

/// What the worker last said about the switch, so a pause or a resume is
/// logged once, when it is first seen, not every pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Seen {
	/// Whether it was last paused; `None` before the first pass read it.
	paused: Option<bool>,
}

/// A move of the switch a pass should say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Change {
	Paused,
	Resumed,
}

impl Seen {
	/// The switch as this pass read it: `Paused` or `Resumed` when it moved
	/// (the first pass says `Paused` only if it starts paused).
	pub(crate) const fn switch(&mut self, paused: bool) -> Option<Change> {
		let was = self.paused.replace(paused);
		match (was, paused) {
			(Some(false) | None, true) => Some(Change::Paused),
			(Some(true), false) => Some(Change::Resumed),
			(Some(false) | None, false) | (Some(true), true) => None,
		}
	}
}

fn say(change: Change) {
	match change {
		Change::Paused => info!("the game feed is paused by the operator: home fetches nothing and keeps what it has"),
		Change::Resumed => info!("the game feed is on again: home follows the games of the week from the scoreboard"),
	}
}

/// Spawn the worker, cancelled through the shared token. The scoreboard is
/// due at once, so a restart catches up without waiting a cadence.
fn spawn(state: &AppState, context: GameFeedContext) {
	let cancel = state.core.cancel_token.clone();
	let db = state.core.shared_db.clone();
	let live = state.realtime.game_live.clone();
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
		let mut seen = Seen::default();
		loop {
			tokio::select! {
				() = cancel.cancelled() => {
					info!("game feed cancelled");
					return;
				}
				_ = ticker.tick() => {
					crate::metrics::game_feed::record_pass_started();
					// The switch, once a pass (`settings::GameFeed`).
					let switch = match settings::read::<GameFeed>(&db).await {
						Ok(standing) => standing.value,
						Err(err) => {
							crate::metrics::instruments::record_error("game_feed", "setting");
							// A switch that does not read is home's storage failing, as
							// a pass that ends on it is (`GameFeedFailing`).
							crate::metrics::game_feed::record_pass(None, schedule.followed(chrono::Utc::now()));
							error!(error = %err, "game feed pass skipped: its following setting could not be read");
							continue;
						}
					};
					crate::metrics::game_feed::record_enabled(switch.following);
					if let Some(change) = seen.switch(!switch.following) {
						say(change);
					}
					// Paused: nothing fetched, and the schedule and every
					// kept row stay as they are. A paused pass follows no
					// game and has not failed.
					if !switch.following {
						crate::metrics::game_feed::record_pass(Some(&pass::Pass::default()), 0);
						continue;
					}
					let result = pass::run_once(&db, &context.fetch, &mut schedule, context.cadence, &chrono::Utc::now).await;
					crate::metrics::game_feed::record_pass(result.as_ref().ok(), schedule.followed(chrono::Utc::now()));
					// What the pass kept is the phone's offer to make
					// again, now rather than at its next tick (GWB4).
					if result.as_ref().is_ok_and(|done| !done.fetched.is_empty()) {
						live.fed();
					}
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

#[cfg(test)]
mod tests {
	use super::{Change, Seen};

	#[test]
	fn the_switch_is_said_when_it_moves_and_not_every_pass() {
		let mut seen = Seen::default();
		assert_eq!(seen.switch(false), None, "on from the start says nothing");
		assert_eq!(seen.switch(false), None);
		assert_eq!(seen.switch(true), Some(Change::Paused));
		assert_eq!(seen.switch(true), None);
		assert_eq!(seen.switch(false), Some(Change::Resumed));
		assert_eq!(Seen::default().switch(true), Some(Change::Paused), "paused from the start is said once");
	}
}
