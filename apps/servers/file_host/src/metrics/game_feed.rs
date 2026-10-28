//! Liveness and work metrics for the game feed (`crate::game_feed`).
//!
//! Shaped like `metrics::transcribe`: most wakes fetch nothing, so
//! `game_feed_last_attempt_timestamp_seconds` moves on every wake, and stale
//! means the task died. Labels are fetch kinds only, never a game or a URL.

use crate::game_feed::pass::Pass;
use std::time::{SystemTime, UNIX_EPOCH};

/// What a fetch can read as, `game_feed_repo::Read::kind` and the kinds of
/// no answer, each started at 0.
const KINDS: [&str; 9] = [
	"scoreboard",
	"summary",
	"rate_limited",
	"status",
	"not_json",
	"not_an_object",
	"timeout",
	"connect",
	"transport",
];

/// Whether this deployment runs the feed: 1 or 0, set once at boot.
pub fn record_enabled(enabled: bool) {
	metrics::gauge!("game_feed_enabled").set(if enabled { 1.0 } else { 0.0 });
}

/// Start every fetch kind at 0, so a kind that never happened reads 0 rather
/// than absent, and the pass as not failing.
pub fn record_started() {
	for kind in KINDS {
		metrics::counter!("game_feed_fetches_total", "kind" => kind).increment(0);
	}
	metrics::counter!("game_feed_plays_added_total").increment(0);
	metrics::gauge!("game_feed_pass_failing").set(0.0);
}

/// Mark a wake.
pub fn record_pass_started() {
	let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |elapsed| elapsed.as_secs_f64());
	metrics::gauge!("game_feed_last_attempt_timestamp_seconds").set(timestamp);
}

/// What a pass did, or that it ended on a storage failure (`None`), and how
/// many games the feed follows now.
pub fn record_pass(pass: Option<&Pass>, followed: usize) {
	metrics::gauge!("game_feed_pass_failing").set(if pass.is_some() { 0.0 } else { 1.0 });
	metrics::gauge!("game_feed_games_followed").set(f64::from(u32::try_from(followed).unwrap_or(u32::MAX)));
	if let Some(pass) = pass {
		for kind in &pass.fetched {
			metrics::counter!("game_feed_fetches_total", "kind" => *kind).increment(1);
		}
		metrics::counter!("game_feed_plays_added_total").increment(u64::try_from(pass.added).unwrap_or(u64::MAX));
	}
}
