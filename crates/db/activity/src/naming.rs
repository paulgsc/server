//! `default_session_name(activities)` — #282 (RCM5), a provisioned session's
//! `name`.
//!
//! Transcribed from `defaultSessionName`
//! (`packages/activity-catalog/src/lib/summary.ts`, `paulgsc/some-ui`), not
//! invented fresh — the same "transcribe, don't invent a second scheme"
//! discipline `activity_repo::provisioning` already established for
//! `CLIENT_MIN_ACTIVITY_DURATION_MS`/`CLIENT_MAX_TOTAL_DURATION_MS`. #282's
//! own acceptance criterion asks for `name` to match what the client's rule
//! would produce "for the same activity list, including `×N` repeats" —
//! this module and its tests are that match, verified against
//! `summary.test.ts`'s own cases rather than a reimplementation guessed at
//! from the issue text alone.
//!
//! The client's version dedupes a full `activityIds` list; the caller here
//! is expected to pass the activities that actually survived
//! [`crate::provision`] (an activity [`crate::provisioning::provision`]
//! omitted — a `NULL` floor, or one that would have pushed the session over
//! [`crate::CLIENT_MAX_TOTAL_DURATION_MS`] — has no business appearing in
//! the name of a session it isn't in), in the same order `recommend()`
//! produced them.

use crate::model::ActivityRecord;
use std::collections::{HashMap, HashSet};

/// A provisioned session's default name.
///
/// Distinct activity names joined with `" + "`, a repeated activity
/// labelled `"Name ×N"` rather than repeated — two Honeycomb blocks with
/// different modes is a valid, expected session shape, not a duplicate to
/// collapse. `"New session"` for an empty list, matching the client's own
/// fallback.
#[must_use]
pub fn default_session_name(activities: &[ActivityRecord]) -> String {
	if activities.is_empty() {
		return "New session".to_owned();
	}

	let mut counts: HashMap<&str, usize> = HashMap::new();
	for activity in activities {
		*counts.entry(activity.id.as_str()).or_insert(0) += 1;
	}

	let mut seen: HashSet<&str> = HashSet::new();
	let mut parts: Vec<String> = Vec::new();
	for activity in activities {
		if !seen.insert(activity.id.as_str()) {
			continue;
		}
		let count = counts.get(activity.id.as_str()).copied().unwrap_or(1);
		parts.push(if count > 1 {
			// `format!` is on `clippy.toml`'s disallowed-macros list (eager
			// allocation ahead of tracing); `write!` into an owned `String`
			// is the same workaround `nudge::waker`'s own test module
			// already uses for the identical lint.
			use std::fmt::Write as _;
			let mut name = activity.name.clone();
			let _ = write!(name, " ×{count}");
			name
		} else {
			activity.name.clone()
		});
	}

	parts.join(" + ")
}

#[cfg(test)]
mod tests {
	use super::default_session_name;
	use crate::model::{ActivityMaturity, ActivityRecord, LayoutTree};

	/// Builds an `ActivityRecord` with only `id`/`name` set meaningfully —
	/// the only two fields this module reads — matching
	/// `provisioning.rs`'s own test helper convention.
	fn activity(id: &str, name: &str) -> ActivityRecord {
		ActivityRecord {
			id: id.to_owned(),
			name: name.to_owned(),
			description: String::new(),
			icon: "hexagon".to_owned(),
			registry_key: id.to_owned(),
			layout_tree: LayoutTree::Study,
			maturity: ActivityMaturity::Ready,
			min_duration_ms: None,
			published_at: "2026-08-01T00:00:00Z".to_owned(),
			version: 1,
			fields: Vec::new(),
			default_config: serde_json::json!({}),
			audio: None,
		}
	}

	/// `summary.test.ts`'s `defaultSessionName` cases, transcribed one for
	/// one.
	#[test]
	fn returns_a_fallback_for_an_empty_session() {
		assert_eq!(default_session_name(&[]), "New session");
	}

	#[test]
	fn joins_distinct_activities_by_name() {
		let activities = [activity("honeycomb", "Hangul Honeycomb"), activity("topik", "TOPIK Study")];
		assert_eq!(default_session_name(&activities), "Hangul Honeycomb + TOPIK Study");
	}

	#[test]
	fn labels_repeated_instances_of_the_same_activity_with_a_count_instead_of_repeating_the_name() {
		let activities = [activity("honeycomb", "Hangul Honeycomb"), activity("honeycomb", "Hangul Honeycomb")];
		assert_eq!(default_session_name(&activities), "Hangul Honeycomb ×2");
	}

	#[test]
	fn counts_repeats_regardless_of_where_they_fall_in_the_order() {
		let activities = [
			activity("honeycomb", "Hangul Honeycomb"),
			activity("topik", "TOPIK Study"),
			activity("honeycomb", "Hangul Honeycomb"),
		];
		assert_eq!(default_session_name(&activities), "Hangul Honeycomb ×2 + TOPIK Study");
	}
}
