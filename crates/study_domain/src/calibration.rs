//! The numbers, and a selector that turns a deficit into something to say.
//!
//! Separated from the vocabulary because they change on different schedules: a
//! new signal class is a product decision, a half-life is an operational one
//! discovered by watching real people.

use crate::signal::{EngagementClass, StudyAction};
use crate::StudyV1;
use chrono::Duration;
use intervention::{Calibration, Deficit, Selector};

/// Release 1's tuning.
///
/// Every number below is a guess that should be revisited against real
/// behaviour. They are written as a total function over a closed enum, so the
/// compiler will point at this file the moment a class is added — which is
/// exactly the reminder wanted.
pub struct StudyCalibration;

impl Calibration<StudyV1> for StudyCalibration {
	fn weight(class: EngagementClass) -> f64 {
		match class {
			// Turning up dominates. Someone who is here is engaged even if
			// they are struggling; someone who is gone is gone.
			EngagementClass::Presence => 1.0,
			EngagementClass::Momentum => 0.7,
			EngagementClass::Mastery => 0.5,
			// Lowest, because staleness is the system's fault, not theirs — it
			// should colour *what* is said more than *whether*.
			EngagementClass::Freshness => 0.4,
		}
	}

	fn half_life(class: EngagementClass) -> Duration {
		match class {
			// Roughly: three quiet days and presence has halved. This is the
			// inactivity model in its entirety.
			EngagementClass::Presence => Duration::days(3),
			EngagementClass::Momentum => Duration::days(5),
			// Understanding fades slowly; a good week should still count a
			// fortnight later.
			EngagementClass::Mastery => Duration::days(14),
			EngagementClass::Freshness => Duration::days(21),
		}
	}

	fn ceiling(_: EngagementClass) -> f64 {
		100.0
	}

	fn recharge_on_intervention(class: EngagementClass) -> f64 {
		match class {
			// A nudge is an attempt at presence, so it buys presence back —
			// enough that the next waker pass finds nothing, not so much that
			// an unanswered nudge counts as a session.
			EngagementClass::Presence => 30.0,
			// Being told about new material is most of the point of hearing
			// about it.
			EngagementClass::Freshness => 20.0,
			// A notification does not finish a session or teach anything.
			EngagementClass::Momentum | EngagementClass::Mastery => 0.0,
		}
	}

	/// Full is 260 (`Σ weight · ceiling`). 110 is a little under halfway: a
	/// person is interrupted once they have drifted meaningfully, not at the
	/// first quiet afternoon.
	const THRESHOLD: f64 = 110.0;

	/// Nothing gets interrupted twice inside a day, whatever the arithmetic
	/// says. The last line of defence against bursts.
	const REFRACTORY: Duration = Duration::hours(20);
}

/// Turns "what is most depleted" into "what to say".
///
/// The dominant deficit picks the message. That is the concrete payoff of
/// [`intervention::Charge`] being a vector: a scalar would reach the same
/// threshold for someone who abandoned a session an hour ago and someone who
/// has been gone a fortnight, and would have nothing to distinguish them by.
pub struct StudySelector {
	/// What the server has prepared, if anything. Without it there is nothing
	/// to point at and the honest answer is silence — a reminder with no
	/// session to open is worse than no reminder.
	pub prepared_session: Option<String>,
}

impl Selector<StudyV1> for StudySelector {
	fn select(&self, deficits: &[Deficit<EngagementClass>]) -> Option<StudyAction> {
		let session_id = self.prepared_session.clone()?;
		let dominant = deficits.first()?.class;

		Some(match dominant {
			EngagementClass::Momentum => StudyAction::ResumeAbandoned { session_id },
			EngagementClass::Mastery => StudyAction::SuggestReview { session_id },
			EngagementClass::Freshness => StudyAction::NewMaterial { session_id },
			EngagementClass::Presence => StudyAction::LessonReady { session_id },
		})
	}
}

#[cfg(test)]
mod tests {
	use super::{StudyCalibration, StudySelector};
	use crate::signal::{EngagementClass, StudyAction, StudySignal};
	use crate::StudyV1;
	use chrono::{Duration, TimeZone as _, Utc};
	use intervention::{Calibration, Charge, Enumerable, Selector};

	fn selector() -> StudySelector {
		StudySelector {
			prepared_session: Some("session-1".to_owned()),
		}
	}

	#[test]
	fn every_class_has_a_positive_weight_and_a_real_half_life() {
		// Both are load-bearing: a non-positive weight breaks the monotonicity
		// `eligible_at` solves against, and a zero half-life zeroes the class.
		for class in EngagementClass::ALL {
			assert!(StudyCalibration::weight(*class) > 0.0, "{class:?}");
			assert!(StudyCalibration::half_life(*class) > Duration::zero(), "{class:?}");
		}
	}

	#[test]
	fn the_threshold_sits_below_full_and_above_empty() {
		let full: f64 = EngagementClass::ALL
			.iter()
			.map(|class| StudyCalibration::weight(*class) * StudyCalibration::ceiling(*class))
			.sum();
		assert!(StudyCalibration::THRESHOLD < full, "a full subject would be instantly eligible");
		// Compared against a runtime value so this stays an assertion about the
		// calibration rather than a constant the compiler folds away.
		let floor: f64 = EngagementClass::ALL.iter().map(|class| StudyCalibration::weight(*class) * 0.0).sum();
		assert!(StudyCalibration::THRESHOLD > floor, "an empty subject would never be eligible");
	}

	#[test]
	fn a_notification_cannot_masquerade_as_a_finished_session() {
		assert!((StudyCalibration::recharge_on_intervention(EngagementClass::Momentum) - 0.0).abs() < f64::EPSILON);
		assert!((StudyCalibration::recharge_on_intervention(EngagementClass::Mastery) - 0.0).abs() < f64::EPSILON);
	}

	#[test]
	fn abandonment_produces_a_resume_prompt_rather_than_a_generic_reminder() {
		let now = Utc.with_ymd_and_hms(2026, 8, 5, 12, 0, 0).unwrap();
		let mut charge = Charge::<StudyV1>::full::<StudyCalibration>(now);
		charge.apply::<StudyCalibration>(
			&StudySignal::SessionAbandoned {
				session_id: "session-1".to_owned(),
				elapsed_ms: 25 * 60_000,
			},
			now,
		);

		let action = selector().select(&charge.deficits::<StudyCalibration>(now)).unwrap();
		assert!(matches!(action, StudyAction::ResumeAbandoned { .. }), "got {action:?}");
	}

	#[test]
	fn poor_scores_ask_for_review_rather_than_more() {
		let now = Utc.with_ymd_and_hms(2026, 8, 5, 12, 0, 0).unwrap();
		let mut charge = Charge::<StudyV1>::full::<StudyCalibration>(now);
		for _ in 0..3 {
			charge.apply::<StudyCalibration>(
				&StudySignal::ScoredBelowTarget {
					activity_id: "a".to_owned(),
					score: 0.1,
				},
				now,
			);
		}

		let action = selector().select(&charge.deficits::<StudyCalibration>(now)).unwrap();
		assert!(matches!(action, StudyAction::SuggestReview { .. }), "got {action:?}");
	}

	#[test]
	fn plain_absence_produces_the_ordinary_lesson_ready() {
		// No signals at all — just time passing, which drains presence fastest.
		let now = Utc.with_ymd_and_hms(2026, 8, 5, 12, 0, 0).unwrap();
		let charge = Charge::<StudyV1>::full::<StudyCalibration>(now);
		let later = now + Duration::days(9);

		let action = selector().select(&charge.deficits::<StudyCalibration>(later)).unwrap();
		assert!(matches!(action, StudyAction::LessonReady { .. }), "got {action:?}");
	}

	#[test]
	fn with_nothing_prepared_there_is_nothing_to_say() {
		let now = Utc.with_ymd_and_hms(2026, 8, 5, 12, 0, 0).unwrap();
		let charge = Charge::<StudyV1>::full::<StudyCalibration>(now);
		let empty = StudySelector { prepared_session: None };
		assert!(empty.select(&charge.deficits::<StudyCalibration>(now)).is_none());
	}
}
