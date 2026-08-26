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
use crate::nudge::presence::PresenceLeases;
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
	pub presence: PresenceLeases,
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

		// An input, not a veto that can run away, and scoped to exactly the
		// context this action is about: `action.session_id()` doubles as the
		// lease's context key (`GetStarted` carries none, so it is never
		// suppressible this way) — see `nudge::presence` for why a
		// site-wide "present" bit was the wrong signal in the first place.
		if self.presence.is_fresh_for(action.session_id(), at) {
			return Err(Suppressed::Present);
		}

		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::{StudyConstraints, Suppressed};
	use crate::nudge::clock::NudgeClock;
	use crate::nudge::presence::PresenceLeases;
	use chrono::{DateTime, Utc};
	use intervention::Admissibility;
	use push_repo::Topic;
	use std::time::Duration;
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
			presence: PresenceLeases::empty(Duration::from_secs(75)),
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
	fn a_fresh_lease_on_the_matching_session_suppresses() {
		let mut present = constraints(vec![Topic::LessonReady]);
		present.presence = PresenceLeases::for_test(vec![("session-1", at(14))], Duration::from_secs(75));
		assert_eq!(present.admit(at(14), &lesson_ready()), Err(Suppressed::Present));
	}

	/// A stale lease — the forgotten second-monitor tab left open all day —
	/// must not silence the nudge indefinitely.
	#[test]
	fn a_stale_lease_does_not_suppress() {
		let mut stale = constraints(vec![Topic::LessonReady]);
		stale.presence = PresenceLeases::for_test(vec![("session-1", at(0))], Duration::from_secs(75));
		assert_eq!(stale.admit(at(14), &lesson_ready()), Ok(()));
	}

	/// The reason this is a lease keyed by context rather than a site-wide
	/// bit: being fresh on some *other* session must not suppress a
	/// notification about this one.
	#[test]
	fn a_fresh_lease_on_a_different_session_does_not_suppress() {
		let mut elsewhere = constraints(vec![Topic::LessonReady]);
		elsewhere.presence = PresenceLeases::for_test(vec![("session-other", at(14))], Duration::from_secs(75));
		assert_eq!(elsewhere.admit(at(14), &lesson_ready()), Ok(()));
	}

	/// `GetStarted` carries no session id — there is no context for a lease
	/// to match, so plain absence can never be suppressed by presence, no
	/// matter what the subject has a fresh lease on.
	#[test]
	fn get_started_is_never_suppressed_by_presence() {
		let mut present = constraints(vec![Topic::LessonReady, Topic::Coaching, Topic::NewMaterial]);
		present.presence = PresenceLeases::for_test(vec![("session-1", at(14))], Duration::from_secs(75));
		assert_eq!(present.admit(at(14), &StudyAction::GetStarted), Ok(()));
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
