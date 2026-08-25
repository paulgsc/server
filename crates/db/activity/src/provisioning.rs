//! `provision(activities)` — #281 (RCM4), the floor-duration rule.
//!
//! #280 (RCM3) decides *which* activities to propose; this decides how long
//! each proposed block runs. The rule, stated in #281's own words: a
//! provisioned activity is scheduled at its **floor**, never its
//! **default** — `defaultMinutes` is tuned for someone who already opened
//! the composer and committed to an activity, and reusing it for someone who
//! has committed to nothing yet is the mistake this story exists to prevent.
//!
//! ## Where the floor actually lives
//!
//! #272 (CAT4) already decided this server never writes `scenes` — only
//! `activities: [{ activityId, config }]`, materialised into playable
//! `scenes` by the client's own `sequenceScenes`
//! (`packages/activity-catalog/src/lib/to-scene-config`, `paulgsc/some-ui`).
//! `sequenceScenes` → `toSceneConfig` → `durationMsFor` reads
//! `config.durationMinutes` when present, and falls back to the catalogue's
//! `defaultConfig.durationMinutes` otherwise. So the floor this story has to
//! enforce is not a field on some server-side `scenes` row (there is none):
//! it is whatever this crate writes into `config.durationMinutes` before
//! `activities` ever reaches the client. Omit it, and the client's own
//! fallback quietly reintroduces exactly the default-duration bug #281
//! exists to prevent.
//!
//! [`provisioned_config`] is therefore `default_config` with exactly one key
//! overridden. #272's own round-trip test doc comment describes
//! `default_config` as handed to the client "verbatim, with no
//! transformation in between" — true of every key #272 had reason to think
//! about, and this module's one deliberate, documented exception to it.
//!
//! ## The two cases #281 names explicitly
//!
//! **The client's floor is not the activity's floor.** Every write path
//! (the Basic composer and the Advanced arrangement editor alike) is
//! checked against `DEFAULT_SESSION_DURATION_POLICY.minActivityDurationMs`
//! (`apps/www/src/lib/session-duration-policy/index.ts`, `paulgsc/some-ui`)
//! by `checkSessionDuration` — a proposal below it would be rejected by the
//! client's own composer the moment someone opened it. So the effective
//! floor is `max(activity_min, client_policy_min)`, not the activity's own
//! minimum alone. [`CLIENT_MIN_ACTIVITY_DURATION_MS`] transcribes that
//! constant; `tests/session_duration_policy_parity.rs` is what fails if the
//! transcription and the client's real value ever diverge.
//!
//! **A `NULL` `min_duration_ms` is decided, not papered over.** #269 made
//! the column nullable for "whichever activity is next to not need one" —
//! `leetype` no longer is (some-ui#1130 gave it a real duration field, see
//! `20260823000700_seed_activities.up.sql`'s comment), but the column stays
//! nullable and the case is still real. The decision: **omit the activity
//! from the proposal entirely**, rather than emit a durationless block.
//! `config.durationMinutes` becoming `null` would not fail loudly — it
//! would hand the client a number it cannot compute a scene from, having
//! silently smuggled a missing measurement past the one function whose job
//! is to refuse that. `None` here means "do not propose this one," and the
//! caller (RCM5, `#282`) is expected to treat it that way: an activity that
//! cannot be timed is not a block that runs for zero minutes, it is a block
//! that cannot be built at all today.
//!
//! No caller yet, same as [`crate::recommend`] before it: RCM5 (`#282`) is
//! what wires this into `nudge::waker::provisioned_session`.

use serde::Serialize;

use crate::model::ActivityRecord;

/// `DEFAULT_SESSION_DURATION_POLICY.minActivityDurationMs`.
///
/// (`apps/www/src/lib/session-duration-policy/index.ts`, `paulgsc/some-ui`,
/// 5 minutes as of this writing) — transcribed rather than derived, since
/// this crate has no way to import TypeScript. Checked against a snapshot
/// of the client's real value by
/// `tests/session_duration_policy_parity.rs`, the same "checked-in fixture,
/// diffed at test time" discipline `tests/catalog_parity.rs` already
/// established for `min_duration_ms` itself.
pub const CLIENT_MIN_ACTIVITY_DURATION_MS: i64 = 5 * 60_000;

/// `DEFAULT_SESSION_DURATION_POLICY.maxTotalDurationMs` (4 hours as of this
/// writing).
///
/// Same transcription-plus-parity-test discipline as
/// [`CLIENT_MIN_ACTIVITY_DURATION_MS`] above.
pub const CLIENT_MAX_TOTAL_DURATION_MS: i64 = 4 * 60 * 60_000;

/// One activity, provisioned.
///
/// `SessionActivity`
/// (`packages/activity-catalog/src/lib/to-scene-config`, `paulgsc/some-ui`)
/// -- `{ activityId, config }` and nothing else, per #272. `camelCase` on
/// the wire for the same reason `ActivityRecord`/`SessionRecord` already
/// are: the client's type is the contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionedActivity {
	pub activity_id: String,
	pub config: serde_json::Value,
}

/// The floor this activity must be scheduled at, in minutes (the unit
/// `ActivityConfigValues.durationMinutes` and `fields`' own `minMinutes`
/// already use).
///
/// `max(activity_min, client_policy_min)`, never the activity's
/// `defaultMinutes`. `None` when `min_duration_ms` is `NULL`: this activity
/// has no derivable duration at all, and this module's decision (see its
/// own doc comment) is to signal "cannot be timed," not to guess one.
#[must_use]
pub fn effective_floor_minutes(activity: &ActivityRecord) -> Option<i64> {
	activity
		.min_duration_ms
		.map(|activity_floor_ms| activity_floor_ms.max(CLIENT_MIN_ACTIVITY_DURATION_MS) / 60_000)
}

/// `activity.default_config` with `durationMinutes` overridden to
/// [`effective_floor_minutes`].
///
/// Every other key (mode, difficulty, level, category, ...) passes through
/// untouched, since only the duration is this story's concern. `None` when
/// the activity has no derivable floor (the `NULL` case), matching
/// [`effective_floor_minutes`].
#[must_use]
pub fn provisioned_config(activity: &ActivityRecord) -> Option<serde_json::Value> {
	let floor_minutes = effective_floor_minutes(activity)?;
	let mut config = activity.default_config.clone();
	if let Some(object) = config.as_object_mut() {
		object.insert("durationMinutes".to_owned(), serde_json::json!(floor_minutes));
	}
	Some(config)
}

/// Turns the recommender's picks into what a provisioned session actually
/// writes to `activities`.
///
/// An activity whose floor cannot be derived (the `NULL` case) is omitted
/// outright -- see this module's doc comment for why silence, not a
/// durationless block, is the right failure mode here.
#[must_use]
pub fn provision(activities: &[ActivityRecord]) -> Vec<ProvisionedActivity> {
	activities
		.iter()
		.filter_map(|activity| {
			provisioned_config(activity).map(|config| ProvisionedActivity {
				activity_id: activity.id.clone(),
				config,
			})
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::{effective_floor_minutes, provision, provisioned_config, CLIENT_MAX_TOTAL_DURATION_MS, CLIENT_MIN_ACTIVITY_DURATION_MS};
	use crate::model::{ActivityMaturity, ActivityRecord, LayoutTree};
	use serde_json::json;

	/// Builds an `ActivityRecord` with the fields this module cares about;
	/// everything else is a placeholder, matching `recommender.rs`'s own
	/// test helper convention.
	fn activity(id: &str, min_duration_ms: Option<i64>, default_config: serde_json::Value) -> ActivityRecord {
		ActivityRecord {
			id: id.to_owned(),
			name: id.to_owned(),
			description: String::new(),
			icon: "hexagon".to_owned(),
			registry_key: id.to_owned(),
			layout_tree: LayoutTree::Study,
			maturity: ActivityMaturity::Ready,
			min_duration_ms,
			published_at: "2026-08-01T00:00:00Z".to_owned(),
			version: 1,
			fields: Vec::new(),
			default_config,
			audio: None,
		}
	}

	/// #281's own table, transcribed directly: each of the four seeded
	/// activities gets its floor, explicitly not its default.
	/// `min_duration_ms`/`default_config` here are the literal values
	/// `20260823000700_seed_activities.up.sql` inserts.
	#[test]
	fn each_seeded_activity_is_provisioned_at_its_floor_not_its_default() {
		let cases = [
			("honeycomb", 300_000, 10, 5),
			("topik", 600_000, 15, 10),
			("interview", 600_000, 20, 10),
			("leetype", 300_000, 10, 5),
		];

		for (id, min_duration_ms, default_minutes, expected_floor_minutes) in cases {
			let record = activity(id, Some(min_duration_ms), json!({"durationMinutes": default_minutes}));
			let config = provisioned_config(&record).unwrap_or_else(|| panic!("`{id}` has a real min_duration_ms and must be provisioned"));
			assert_eq!(
				config["durationMinutes"],
				json!(expected_floor_minutes),
				"`{id}` must be provisioned at its floor ({expected_floor_minutes}m), not its default ({default_minutes}m)"
			);
			assert_ne!(
				config["durationMinutes"],
				json!(default_minutes),
				"`{id}`'s provisioned duration must not equal its default"
			);
		}
	}

	/// "The client's floor is not the activity's floor": an activity whose
	/// own minimum is below `CLIENT_MIN_ACTIVITY_DURATION_MS` is still
	/// floored at the client's number, not its own lower one -- otherwise
	/// the client's own composer would reject the proposal on open.
	#[test]
	fn the_client_policy_floor_wins_when_it_is_higher_than_the_activitys_own_minimum() {
		let one_minute_activity = activity("hypothetical", Some(60_000), json!({}));
		assert_eq!(effective_floor_minutes(&one_minute_activity), Some(CLIENT_MIN_ACTIVITY_DURATION_MS / 60_000));
	}

	/// The activity's own minimum wins when it is already above the
	/// client's floor -- `max`, not "always the client's number."
	#[test]
	fn the_activitys_own_minimum_wins_when_it_is_already_above_the_client_floor() {
		let ten_minute_activity = activity("hypothetical", Some(600_000), json!({}));
		assert_eq!(effective_floor_minutes(&ten_minute_activity), Some(10));
	}

	/// #281's `leetype` scenario, kept alive syntheically: no seeded
	/// activity has a `NULL` `min_duration_ms` any more (some-ui#1130), but
	/// the column stays nullable "for whichever activity is next" per
	/// #269's own comment, and this module's decision -- omit rather than
	/// guess -- has to hold for that activity too, whichever it turns out
	/// to be.
	#[test]
	fn an_activity_with_no_derivable_duration_is_omitted_rather_than_given_a_durationless_block() {
		let undated = activity("hypothetical-no-duration-field", None, json!({}));
		assert_eq!(effective_floor_minutes(&undated), None);
		assert_eq!(provisioned_config(&undated), None);
		assert_eq!(
			provision(std::slice::from_ref(&undated)),
			Vec::new(),
			"an un-timeable activity must not appear in the provisioned list at all"
		);
	}

	/// Every other key on `default_config` passes through untouched --
	/// #272's "verbatim" claim holds for everything except the one key this
	/// story has a documented reason to override.
	#[test]
	fn every_default_config_key_other_than_duration_minutes_passes_through_unchanged() {
		let record = activity("honeycomb", Some(300_000), json!({"mode": "completion", "difficulty": "standard", "durationMinutes": 10}));
		// `.unwrap()`, not `.expect()`: `clippy.toml`'s `allow-unwrap-in-tests`
		// covers the former only, matching this crate's other test modules.
		let config = provisioned_config(&record).unwrap();
		assert_eq!(config["mode"], json!("completion"));
		assert_eq!(config["difficulty"], json!("standard"));
		assert_eq!(config["durationMinutes"], json!(5));
	}

	/// `maxTotalDurationMs` (4 hours): trivially true at `k=2` per #281's own
	/// acceptance criterion, asserted here against all four seeded
	/// activities at once (`k=4`) so it stays true if `k` ever moves, not
	/// just at today's `DEFAULT_RECOMMENDATION_COUNT`.
	#[test]
	fn the_combined_floor_of_every_seeded_activity_stays_under_the_session_cap() {
		let catalogue = vec![
			activity("honeycomb", Some(300_000), json!({"durationMinutes": 10})),
			activity("topik", Some(600_000), json!({"durationMinutes": 15})),
			activity("interview", Some(600_000), json!({"durationMinutes": 20})),
			activity("leetype", Some(300_000), json!({"durationMinutes": 10})),
		];

		let provisioned = provision(&catalogue);
		assert_eq!(provisioned.len(), 4, "none of the four seeded activities has a NULL floor today");

		let total_ms: i64 = provisioned.iter().map(|activity| activity.config["durationMinutes"].as_i64().unwrap() * 60_000).sum();
		assert!(
			total_ms <= CLIENT_MAX_TOTAL_DURATION_MS,
			"combined floor duration {total_ms}ms exceeds the {CLIENT_MAX_TOTAL_DURATION_MS}ms session cap"
		);
	}

	/// `provision` filters, it does not reorder or otherwise editorialise --
	/// same activities in, same activities out (minus any `NULL`-floor
	/// ones), in the order `recommend()` already decided.
	#[test]
	fn provision_preserves_the_recommenders_order() {
		let catalogue = vec![activity("topik", Some(600_000), json!({})), activity("honeycomb", Some(300_000), json!({}))];
		let provisioned = provision(&catalogue);
		let ids: Vec<&str> = provisioned.iter().map(|activity| activity.activity_id.as_str()).collect();
		assert_eq!(ids, vec!["topik", "honeycomb"]);
	}
}
