//! Emits a real provisioned session's `activities` -- RCM3's `recommend()`
//! plus RCM4's `provision()`, run against a literal transcription of the
//! seeded catalogue -- as JSON on stdout. `paulgsc/server#281`.
//!
//! ```sh
//! cargo run -q --bin dump-proposed-session > proposed-session.snapshot.json
//! ```
//!
//! Mirrors `dump-routes` (this same directory): no `Config`, no database,
//! nothing that would make this annoying enough to go stale. The four
//! activities below are transcribed from
//! `20260823000700_seed_activities.up.sql` (`min_duration_ms`,
//! `default_config`) -- `tests/catalog_parity.rs` already guards that
//! migration against the client, so this bin only needs to be faithful to
//! *it*, not to re-derive anything.
//!
//! The client repo checks the output in
//! (`apps/www/src/lib/session-duration-policy/testdata/`) and a test there
//! feeds it through the real `sequenceScenes` and `checkSessionDuration` --
//! #281's own acceptance criterion that this is verified "by a fixture
//! consumed on the client side, not by re-implementing the check here."
//! Same by-hand copy discipline as `dump-routes`/`dump-activity-catalog.ts`;
//! see either's doc comment for why nothing automates the bridge yet.
//!
//! Regenerate and re-copy whenever the seed migration or the recommender's
//! rules change in a way that would move which two activities this
//! produces or what they're scheduled at.

use activity_repo::{provision, recommend, ActivityMaturity, ActivityRecord, LayoutTree, DEFAULT_RECOMMENDATION_COUNT};
use chrono::{TimeZone, Utc};
use std::io::{self, Write};

/// The four seeded activities, transcribed from
/// `20260823000700_seed_activities.up.sql` -- id, maturity, `published_at`,
/// `min_duration_ms`, and `default_config` only, since those are all
/// `recommend()`/`provision()` read.
fn seeded_catalogue() -> Vec<ActivityRecord> {
	let activity = |id: &str, maturity: ActivityMaturity, published_at: &str, min_duration_ms: i64, default_config: serde_json::Value| ActivityRecord {
		id: id.to_owned(),
		name: id.to_owned(),
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
			ActivityMaturity::Ready,
			"2026-08-23T00:00:00Z",
			300_000,
			serde_json::json!({"mode": "completion", "difficulty": "standard", "durationMinutes": 10}),
		),
		activity(
			"topik",
			ActivityMaturity::Preview,
			"2026-08-23T00:00:00Z",
			600_000,
			serde_json::json!({"level": "beginner", "durationMinutes": 15}),
		),
		activity(
			"interview",
			ActivityMaturity::Early,
			"2026-08-23T00:00:00Z",
			600_000,
			serde_json::json!({"level": "mid", "category": "technical", "durationMinutes": 20}),
		),
		activity(
			"leetype",
			ActivityMaturity::Ready,
			"2026-08-23T00:00:00Z",
			300_000,
			serde_json::json!({"durationMinutes": 10}),
		),
	]
}

fn main() -> io::Result<()> {
	let catalogue = seeded_catalogue();
	// A fixed subject and clock: this fixture has to be reproducible on
	// every regen, the same "same subject, same day" guarantee
	// `recommender.rs`'s own tests pin against a literal expected value for.
	let now = Utc.with_ymd_and_hms(2026, 8, 25, 9, 0, 0).unwrap();
	let picks = recommend("fixture-subject", DEFAULT_RECOMMENDATION_COUNT, &catalogue, &[], None, now);
	let provisioned = provision(&picks);

	let stdout = io::stdout();
	let mut out = stdout.lock();
	// `to_writer_pretty` rather than `to_string`: `clippy.toml` disallows the
	// latter, and streaming avoids materialising the document twice -- same
	// reasoning `dump_routes.rs` gives for its own call.
	serde_json::to_writer_pretty(&mut out, &provisioned).map_err(io::Error::other)?;
	out.write_all(b"\n")?;
	out.flush()
}
