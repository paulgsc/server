//! `recommend(subject, k)` — #280 (RCM3), the recommender.
//!
//! Part of #257's cold-start work: the waker's `NothingToSay` arm (#279,
//! RCM2) already provisions an empty session (`activities: Vec::new()`);
//! this is what decides what goes in it, once #282 (RCM5) wires the two
//! together. Wiring is deliberately out of scope here, per #279's and
//! #280's own out-of-scope notes.
//!
//! There is no performance data yet -- #258 (`TEL1`) will produce some, but
//! today there is none -- so this is not a model, it is three named,
//! ordered rules, exactly the discipline [`crate::model::ActivityMaturity`]'s
//! own doc comment already promises: "the recommender filters on this
//! directly." The three axes, in the order they're allowed to disagree:
//!
//! 1. **Newness dominates** ([`WEIGHT_NEWNESS`]) -- an activity this subject
//!    has never played, or one republished since their last session,
//!    outranks everything else.
//! 2. **Low engagement lifts, it does not lower** ([`WEIGHT_ENGAGEMENT`],
//!    [`engagement_lift`]) -- an abandoned or poorly-scored activity is
//!    unfinished or unlearned, not a reason to stop offering it.
//! 3. **Otherwise, a seeded shuffle** ([`shuffle_ranks`]) -- deterministic
//!    per `(subject, day)`, so a person who dismisses a proposal and
//!    reopens the app does not get a different one.
//!
//! This deliberately disagrees with the client's own ranking --
//! `rankActivitiesWithScores` in `packages/activity-catalog/src/lib/rank.ts`
//! (`paulgsc/some-ui`) -- which weights *recency of play* highest. That
//! ranker answers "what do I want to open right now" for a person browsing
//! a dashboard, where recency is a good predictor. This one answers "what
//! should I do next" for a person who asked for nothing, where recency of
//! their *own* play is a poor predictor and newness of the *content* is a
//! better one. Both are correct for the question they answer; this note and
//! `rank.ts`'s own are what stop someone "fixing" one to match the other.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use chrono::{DateTime, NaiveDate, Utc};
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;

use crate::model::{ActivityMaturity, ActivityRecord};

/// How a subject relates to one activity, exactly as #258 (`TEL1`,
/// `activity_outcome`) will eventually supply it.
///
/// Until that table exists -- and until #289 flips the flag that makes it
/// the default input here, per #280's own out-of-scope note -- production
/// callers pass an empty slice. No activity has an entry, which
/// [`engagement_lift`] and [`recommend`]'s newness check both read as
/// "never played" -- the correct cold-start default per #257, not a
/// degenerate one.
#[derive(Debug, Clone)]
pub struct ActivityHistory {
	pub activity_id: String,
	pub outcome: ActivityOutcome,
}

/// What happened the last time this subject touched this activity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ActivityOutcome {
	/// Started, never finished. Unfinished, not failed.
	Abandoned,
	/// Finished, with `score` in `0.0..=1.0`. A low score reads as
	/// "unlearned, worth repeating" under axis 2, never as a mark against
	/// the activity.
	Completed { score: f64 },
}

/// How many activities a provisioned session proposes.
///
/// Two, per #280/#257: "one is a thin session, three is a commitment."
/// Not read by [`recommend`] itself -- `k` is a parameter there so a test
/// can ask for any number -- this is what a production caller (RCM5, #282)
/// is expected to pass.
pub const DEFAULT_RECOMMENDATION_COUNT: usize = 2;

/// Newness dominates every other axis. Deliberately the same *number* as
/// the client's own `WEIGHT_RECENCY` (`rank.ts`), opposite *meaning* -- see
/// this module's own doc comment for why the two rankers disagree on
/// purpose.
const WEIGHT_NEWNESS: f64 = 8.0;

/// Zero history, poor performance, and high abandonment all lift a
/// candidate under this weight (see [`engagement_lift`]). Large enough that
/// a maxed-out lift (`1.0`) can never overtake one point of newness; small
/// enough that newness always wins outright when the two disagree.
const WEIGHT_ENGAGEMENT: f64 = 4.0;

/// The `k` best activities to propose to `subject` right now.
///
/// Pure over its inputs plus `now`: the same `catalogue`/`history`/
/// `last_session_at` on the same UTC day always produces the same order --
/// testable at any hour, like [`intervention::Selector`]'s implementations
/// before it, and a person who dismisses a proposal and reopens the app
/// does not get a different one.
///
/// `catalogue` should already be a bounded read (see
/// [`crate::repository::ActivityRepository::list`]) -- this function does
/// no I/O and enforces no ceiling of its own. `maturity = Early` candidates
/// are excluded outright, per #280: a construction zone is a poor thing to
/// propose unprompted, unlike a menu someone is browsing where it can just
/// rank low.
#[must_use]
pub fn recommend(
	subject: &str,
	k: usize,
	catalogue: &[ActivityRecord],
	history: &[ActivityHistory],
	last_session_at: Option<DateTime<Utc>>,
	now: DateTime<Utc>,
) -> Vec<ActivityRecord> {
	let shuffle_rank = shuffle_ranks(subject, now.date_naive(), catalogue.len());

	let mut candidates: Vec<(usize, f64)> = catalogue
		.iter()
		.enumerate()
		.filter(|(_, activity)| activity.maturity != ActivityMaturity::Early)
		.map(|(index, activity)| (index, score(activity, history, last_session_at)))
		.collect();

	candidates.sort_by(|(a_index, a_score), (b_index, b_score)| b_score.total_cmp(a_score).then(shuffle_rank[*a_index].cmp(&shuffle_rank[*b_index])));

	candidates.into_iter().take(k).map(|(index, _)| catalogue[index].clone()).collect()
}

fn score(activity: &ActivityRecord, history: &[ActivityHistory], last_session_at: Option<DateTime<Utc>>) -> f64 {
	let entry = history.iter().find(|h| h.activity_id == activity.id);
	let newness = if is_new(activity, entry, last_session_at) { WEIGHT_NEWNESS } else { 0.0 };
	WEIGHT_ENGAGEMENT.mul_add(engagement_lift(entry), newness)
}

/// Axis 1: an activity this subject has never played is always new to them.
/// One they *have* played only counts as new again if its catalogue entry
/// was published after their last session -- the content itself changed
/// since they were last here.
fn is_new(activity: &ActivityRecord, entry: Option<&ActivityHistory>, last_session_at: Option<DateTime<Utc>>) -> bool {
	if entry.is_none() {
		return true;
	}
	match (published_at(activity), last_session_at) {
		(Some(published), Some(last_session)) => published > last_session,
		_ => false,
	}
}

/// `published_at` is a stored `TEXT` column (RFC 3339), same as every other
/// timestamp this schema keeps as a string. A row that fails to parse
/// contributes nothing to axis 1 rather than being refused outright --
/// unlike `ActivityMaturity::parse`/`LayoutTree::parse`, a malformed
/// timestamp here is a reason to rank this one candidate conservatively,
/// not a reason to fail the whole recommendation.
fn published_at(activity: &ActivityRecord) -> Option<DateTime<Utc>> {
	DateTime::parse_from_rfc3339(&activity.published_at).ok().map(|dt| dt.with_timezone(&Utc))
}

/// Axis 2, deliberately counter-intuitive: zero history, an abandoned
/// attempt, and a poor score all lift equally toward `1.0` -- an unfinished
/// or unlearned activity is worth offering again, not worth hiding. Only a
/// *good* completed score pulls this down.
fn engagement_lift(entry: Option<&ActivityHistory>) -> f64 {
	match entry.map(|h| h.outcome) {
		None | Some(ActivityOutcome::Abandoned) => 1.0,
		Some(ActivityOutcome::Completed { score }) => 1.0 - score.clamp(0.0, 1.0),
	}
}

/// Axis 3: a deterministic, per-`(subject, day)` ordering over `0..len`,
/// via Fisher-Yates over a seeded PRNG -- not `rand::thread_rng`, so the
/// same subject asking again the same day gets the same proposal rather
/// than a slot machine. `result[i]` is where original index `i` landed in
/// the shuffle, so callers can sort by it directly as a tie-break key.
fn shuffle_ranks(subject: &str, day: NaiveDate, len: usize) -> Vec<usize> {
	let mut hasher = DefaultHasher::new();
	subject.hash(&mut hasher);
	day.hash(&mut hasher);
	let mut rng = StdRng::seed_from_u64(hasher.finish());

	let mut order: Vec<usize> = (0..len).collect();
	order.shuffle(&mut rng);

	let mut rank = vec![0; len];
	for (position, index) in order.into_iter().enumerate() {
		rank[index] = position;
	}
	rank
}

#[cfg(test)]
mod tests {
	use chrono::{TimeZone, Utc};

	use super::{recommend, ActivityHistory, ActivityOutcome};
	use crate::model::{ActivityMaturity, ActivityRecord, LayoutTree};

	fn activity(id: &str, maturity: ActivityMaturity, published_at: &str) -> ActivityRecord {
		ActivityRecord {
			id: id.to_owned(),
			name: id.to_owned(),
			description: String::new(),
			icon: "hexagon".to_owned(),
			registry_key: id.to_owned(),
			layout_tree: LayoutTree::Study,
			maturity,
			min_duration_ms: None,
			published_at: published_at.to_owned(),
			version: 1,
			fields: Vec::new(),
			default_config: serde_json::json!({}),
			audio: None,
		}
	}

	fn ids(records: &[ActivityRecord]) -> Vec<&str> {
		records.iter().map(|record| record.id.as_str()).collect()
	}

	/// A zero-history subject -- empty `history`, no `last_session_at` --
	/// still gets a full, deterministic, non-empty selection: every
	/// activity ties on both axes, so the seeded shuffle alone decides the
	/// order. The exact list is pinned rather than merely checked for
	/// shape, per #280's own acceptance criterion -- and pinning it here
	/// is what the "identical every time" property (this test's third
	/// assertion) exists to make safe to rely on.
	#[test]
	fn a_zero_history_subject_gets_a_deterministic_non_empty_selection() {
		let catalogue = vec![
			activity("honeycomb", ActivityMaturity::Ready, "2026-08-01T00:00:00Z"),
			activity("topik", ActivityMaturity::Ready, "2026-08-02T00:00:00Z"),
			activity("interview", ActivityMaturity::Preview, "2026-08-03T00:00:00Z"),
		];
		let now = Utc.with_ymd_and_hms(2026, 8, 24, 12, 0, 0).unwrap();

		let first = recommend("subject-1", 2, &catalogue, &[], None, now);
		let second = recommend("subject-1", 2, &catalogue, &[], None, now);

		assert_eq!(ids(&first), vec!["topik", "honeycomb"], "the exact selection for this fixed subject/day/catalogue");
		assert_eq!(ids(&first), ids(&second), "same subject, same day: repeated calls must be identical");
	}

	/// Same subject, same UTC day, repeated calls: identical output, tested
	/// against a catalogue where axes 1 and 2 actually disagree (so this
	/// isn't just re-testing the all-tied case above).
	#[test]
	fn the_same_subject_and_day_produce_the_same_recommendation_every_time() {
		let catalogue = vec![
			activity("honeycomb", ActivityMaturity::Ready, "2026-08-01T00:00:00Z"),
			activity("topik", ActivityMaturity::Ready, "2026-08-20T00:00:00Z"),
			activity("interview", ActivityMaturity::Ready, "2026-08-21T00:00:00Z"),
			activity("leetype", ActivityMaturity::Ready, "2026-08-22T00:00:00Z"),
		];
		let history = vec![ActivityHistory {
			activity_id: "topik".to_owned(),
			outcome: ActivityOutcome::Abandoned,
		}];
		let now = Utc.with_ymd_and_hms(2026, 8, 24, 9, 0, 0).unwrap();

		let first = recommend("subject-2", 3, &catalogue, &history, None, now);
		let second = recommend("subject-2", 3, &catalogue, &history, None, now);

		assert_eq!(ids(&first), ids(&second));
	}

	/// Different UTC day, same everything else: the shuffle component of
	/// the ranking (usually) changes the order. Checked against a spread of
	/// days rather than one fixed pair, so this isn't hostage to one day's
	/// hash landing on a coincidental repeat.
	#[test]
	fn a_different_day_usually_produces_a_different_recommendation() {
		let catalogue = vec![
			activity("honeycomb", ActivityMaturity::Ready, "2026-08-01T00:00:00Z"),
			activity("topik", ActivityMaturity::Ready, "2026-08-01T00:00:00Z"),
			activity("interview", ActivityMaturity::Ready, "2026-08-01T00:00:00Z"),
			activity("leetype", ActivityMaturity::Ready, "2026-08-01T00:00:00Z"),
		];
		let day_zero = Utc.with_ymd_and_hms(2026, 8, 24, 12, 0, 0).unwrap();
		let baseline = recommend("subject-3", 4, &catalogue, &[], None, day_zero);

		let differs = (1..=7).any(|offset| {
			let later = day_zero + chrono::Duration::days(offset);
			let candidate = recommend("subject-3", 4, &catalogue, &[], None, later);
			ids(&candidate) != ids(&baseline)
		});

		assert!(differs, "expected at least one of the next 7 days to reorder a 4-activity, all-tied catalogue");
	}

	/// The whole point of axis 2: given one activity abandoned and another
	/// completed well, the abandoned one ranks higher, not lower. Both
	/// activities have history entries (so axis 1 -- newness -- ties them
	/// at zero) and no `last_session_at`, isolating axis 2's effect.
	#[test]
	fn an_abandoned_activity_outranks_a_well_completed_one() {
		let catalogue = vec![
			activity("well-completed", ActivityMaturity::Ready, "2026-01-01T00:00:00Z"),
			activity("abandoned", ActivityMaturity::Ready, "2026-01-01T00:00:00Z"),
		];
		let history = vec![
			ActivityHistory {
				activity_id: "well-completed".to_owned(),
				outcome: ActivityOutcome::Completed { score: 0.95 },
			},
			ActivityHistory {
				activity_id: "abandoned".to_owned(),
				outcome: ActivityOutcome::Abandoned,
			},
		];
		let now = Utc.with_ymd_and_hms(2026, 8, 24, 12, 0, 0).unwrap();

		let result = recommend("subject-4", 2, &catalogue, &history, None, now);

		assert_eq!(ids(&result), vec!["abandoned", "well-completed"], "abandonment must lift, not lower, a candidate's rank");
	}

	/// `maturity = Early` is excluded outright, even when it would otherwise
	/// win on every axis -- a construction zone is a poor thing to propose
	/// unprompted.
	#[test]
	fn an_early_maturity_activity_is_never_recommended() {
		let catalogue = vec![
			activity("under-construction", ActivityMaturity::Early, "2026-08-24T00:00:00Z"),
			activity("finished", ActivityMaturity::Ready, "2020-01-01T00:00:00Z"),
		];
		let now = Utc.with_ymd_and_hms(2026, 8, 24, 12, 0, 0).unwrap();

		let result = recommend("subject-5", 2, &catalogue, &[], None, now);

		assert_eq!(
			ids(&result),
			vec!["finished"],
			"an early-maturity activity must never appear, however new or untouched it is"
		);
	}
}
