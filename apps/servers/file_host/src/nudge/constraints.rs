//! May we interrupt, right now?
//!
//! Everything the previous design called "the policy" lives here, and the
//! demotion is the point. Quiet hours, presence, opt-in — every one of those
//! guards answers *may we*, and not one answers *why*. A design where they are
//! the whole policy has no warrant layer at all, so something has to supply
//! one, and the usual substitute is a clock: the tick happens, therefore we
//! consider notifying.
//!
//! Warrant now comes from `intervention::Charge` falling to a threshold. These
//! are constraints on acting, which is what they always were.

use crate::nudge::clock::{is_within_quiet_hours, NudgeClock};
use crate::nudge::payload::topic_for;
use crate::nudge::presence::Presence;
use chrono::{DateTime, Utc};
use intervention::Admissibility;
use push_repo::Topic;
use study_domain::{StudyAction, StudyV1};

/// Why an interruption that was warranted did not happen.
///
/// Named rather than boolean because a system that goes quiet has to be able to
/// say why. This is the first question anyone asks, and "the reminder feature
/// is broken" and "you were mid-session" are indistinguishable without it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suppressed {
	/// The deployment has notifications turned off entirely.
	Disabled,
	/// Nobody consented to this kind of interruption.
	NotConsented,
	/// Local quiet hours.
	QuietHours,
	/// Someone is looking at the dashboard right now.
	Present,
}

impl Suppressed {
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Disabled => "disabled",
			Self::NotConsented => "not-consented",
			Self::QuietHours => "quiet-hours",
			Self::Present => "present",
		}
	}
}

/// The constraint set, evaluated per decision.
///
/// Holds a snapshot rather than reaching for live state, so the decision stays
/// a pure function of its inputs and can be tested at any hour of any day.
pub struct StudyConstraints {
	pub clock: NudgeClock,
	pub enabled: bool,
	pub quiet_hours_start: u32,
	pub quiet_hours_end: u32,
	pub presence: Presence,
	/// What this subject's devices actually agreed to receive.
	pub consented_topics: Vec<Topic>,
}

impl Admissibility<StudyV1> for StudyConstraints {
	type Reason = Suppressed;

	fn admit(&self, at: DateTime<Utc>, action: &StudyAction) -> Result<(), Suppressed> {
		if !self.enabled {
			return Err(Suppressed::Disabled);
		}

		// Before anything else about timing: consent is not a preference to be
		// weighed, it is a precondition. The null case is silence.
		if !self.consented_topics.contains(&topic_for(action)) {
			return Err(Suppressed::NotConsented);
		}

		if is_within_quiet_hours(self.clock.local_hour(at), self.quiet_hours_start, self.quiet_hours_end) {
			return Err(Suppressed::QuietHours);
		}

		// An input, not a veto that can run away: `Presence` only reports true
		// for a connection that is active, unstale, and recently heard from —
		// see `nudge::presence` for why a pinned background tab must not be
		// able to silence this forever.
		if self.presence.is_present() {
			return Err(Suppressed::Present);
		}

		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::{StudyConstraints, Suppressed};
	use crate::nudge::clock::NudgeClock;
	use crate::nudge::presence::Presence;
	use chrono::{DateTime, Utc};
	use intervention::Admissibility;
	use push_repo::Topic;
	use study_domain::StudyAction;

	fn at(hour: u32) -> DateTime<Utc> {
		crate::nudge::clock::parse_timestamp(&format!("2026-08-05T{hour:02}:00:00Z")).unwrap()
	}

	fn constraints(topics: Vec<Topic>) -> StudyConstraints {
		StudyConstraints {
			clock: NudgeClock::resolve(Some("UTC")).0,
			enabled: true,
			quiet_hours_start: 22,
			quiet_hours_end: 8,
			presence: Presence { connected: 0, live: 0 },
			consented_topics: topics,
		}
	}

	fn lesson_ready() -> StudyAction {
		StudyAction::LessonReady {
			session_id: "session-1".to_owned(),
		}
	}

	#[test]
	fn a_consented_action_outside_quiet_hours_is_admitted() {
		assert_eq!(constraints(vec![Topic::LessonReady]).admit(at(14), &lesson_ready()), Ok(()));
	}

	#[test]
	fn consent_is_a_precondition_not_a_preference() {
		// Everything else is fine; they simply never agreed to this one.
		assert_eq!(constraints(vec![Topic::Coaching]).admit(at(14), &lesson_ready()), Err(Suppressed::NotConsented));
		assert_eq!(constraints(Vec::new()).admit(at(14), &lesson_ready()), Err(Suppressed::NotConsented));
	}

	#[test]
	fn consent_outranks_quiet_hours_so_the_reason_names_the_deeper_problem() {
		// Both would suppress. Reporting "quiet hours" would have someone
		// waiting until morning for a notification that was never permitted.
		assert_eq!(constraints(Vec::new()).admit(at(23), &lesson_ready()), Err(Suppressed::NotConsented));
	}

	#[test]
	fn quiet_hours_wrap_midnight() {
		let allowed = constraints(vec![Topic::LessonReady]);
		assert_eq!(allowed.admit(at(23), &lesson_ready()), Err(Suppressed::QuietHours));
		assert_eq!(allowed.admit(at(3), &lesson_ready()), Err(Suppressed::QuietHours));
		assert_eq!(allowed.admit(at(8), &lesson_ready()), Ok(()));
	}

	#[test]
	fn a_live_dashboard_suppresses_and_a_stale_socket_does_not() {
		let mut live = constraints(vec![Topic::LessonReady]);
		live.presence = Presence { connected: 1, live: 1 };
		assert_eq!(live.admit(at(14), &lesson_ready()), Err(Suppressed::Present));

		// The forgotten second-monitor tab: a connection is held, none is fresh.
		let mut zombie = constraints(vec![Topic::LessonReady]);
		zombie.presence = Presence { connected: 3, live: 0 };
		assert_eq!(zombie.admit(at(14), &lesson_ready()), Ok(()));
	}

	#[test]
	fn coaching_and_new_material_need_their_own_grants() {
		let coaching_only = constraints(vec![Topic::Coaching]);
		let resume = StudyAction::ResumeAbandoned {
			session_id: "session-1".to_owned(),
		};
		let fresh = StudyAction::NewMaterial {
			session_id: "session-1".to_owned(),
		};

		assert_eq!(coaching_only.admit(at(14), &resume), Ok(()));
		assert_eq!(coaching_only.admit(at(14), &fresh), Err(Suppressed::NotConsented));
	}
}
