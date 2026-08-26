//! Emits a real provisioned session -- `#282`'s `materialize_provisioned_session`,
//! run against a literal transcription of the seeded catalogue -- as JSON on
//! stdout.
//!
//! ```sh
//! cargo run -q --bin dump-provisioned-session > provisioned-session.snapshot.json
//! ```
//!
//! Mirrors `dump-proposed-session` (this same directory), which this bin
//! shares its fixed subject, clock, and seeded catalogue with -- #282's own
//! acceptance criterion is that a provisioned session "round-trips through
//! `SessionRepository` and deserialises into the client's `SessionRecord`
//! without error -- verified by a fixture the client repo consumes, not by
//! a hand-copied type." Calling `file_host::nudge::waker::
//! materialize_provisioned_session` directly, rather than re-deriving its
//! output here, is what makes this fixture trustworthy: it is the exact
//! function `nudge::waker::consider` calls in production, not a
//! reimplementation of it.
//!
//! The client repo checks the output in
//! (`apps/www/src/lib/tenant/testdata/`) and a test there round-trips it
//! through the real `SessionRecord` type and the real `sequenceScenes`/
//! `defaultSessionName`/`totalDurationOfScenes` functions -- same by-hand
//! copy discipline `dump-routes`/`dump-proposed-session` already established;
//! see either's doc comment for why nothing automates the bridge yet.
//!
//! Regenerate and re-copy whenever the seed migration or the recommender's
//! rules change in a way that would move which activities this produces,
//! what they're scheduled at, or what they're named.

use activity_repo::{ActivityMaturity, ActivityRecord, LayoutTree};
use chrono::{TimeZone, Utc};
use file_host::nudge::waker::materialize_provisioned_session;
use std::io::{self, Write};

/// The four seeded activities, transcribed from
/// `20260823000700_seed_activities.up.sql` -- identical to
/// `dump_proposed_session.rs`'s own copy, since both bins have to agree on
/// what the seed migration actually contains and neither can import
/// `sqlx::query!`-verified rows without a live database.
fn seeded_catalogue() -> Vec<ActivityRecord> {
	let activity = |id: &str, name: &str, maturity: ActivityMaturity, published_at: &str, min_duration_ms: i64, default_config: serde_json::Value| ActivityRecord {
		id: id.to_owned(),
		name: name.to_owned(),
		description: String::new(),
		icon: String::new(),
		registry_key: id.to_owned(),
		layout_tree: LayoutTree::Study,
		maturity,
		min_duration_ms: Some(min_duration_ms),
		published_at: published_at.to_owned(),
		version: 1,
		fields: Vec::new(),
		default_config,
		audio: None,
	};

	vec![
		activity(
			"honeycomb",
			"Hangul Honeycomb",
			ActivityMaturity::Ready,
			"2026-08-23T00:00:00Z",
			300_000,
			serde_json::json!({"mode": "completion", "difficulty": "standard", "durationMinutes": 10}),
		),
		activity(
			"topik",
			"TOPIK Study",
			ActivityMaturity::Preview,
			"2026-08-23T00:00:00Z",
			600_000,
			serde_json::json!({"level": "beginner", "durationMinutes": 15}),
		),
		activity(
			"interview",
			"Interview Prep",
			ActivityMaturity::Early,
			"2026-08-23T00:00:00Z",
			600_000,
			serde_json::json!({"level": "mid", "category": "technical", "durationMinutes": 20}),
		),
		activity(
			"leetype",
			"LeetType",
			ActivityMaturity::Ready,
			"2026-08-23T00:00:00Z",
			300_000,
			serde_json::json!({"durationMinutes": 10}),
		),
	]
}

fn main() -> io::Result<()> {
	let catalogue = seeded_catalogue();
	// Same fixed subject and clock as `dump_proposed_session.rs`, so the two
	// fixtures stay comparable and this one reproduces on every regen.
	let now = Utc.with_ymd_and_hms(2026, 8, 25, 9, 0, 0).unwrap();
	let record = materialize_provisioned_session("session-fixture-provisioned".to_owned(), "fixture-subject", &catalogue, now);

	let stdout = io::stdout();
	let mut out = stdout.lock();
	// `to_writer_pretty` rather than `to_string`: `clippy.toml` disallows the
	// latter, and streaming avoids materialising the document twice -- same
	// reasoning `dump_proposed_session.rs` gives for its own call.
	serde_json::to_writer_pretty(&mut out, &record).map_err(io::Error::other)?;
	out.write_all(b"\n")?;
	out.flush()
}
