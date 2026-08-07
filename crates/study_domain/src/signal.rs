use intervention::{Enumerable, Signal};
use serde::{Deserialize, Serialize};

/// The four dimensions of "is this person still with us".
///
/// Discriminants are **append-only**. They are written to durable storage, and
/// reusing one silently reinterprets every row a previous release wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EngagementClass {
	/// Turning up at all. Plain absence is what drains this one.
	Presence,
	/// Finishing what they start.
	Momentum,
	/// Whether the material is landing.
	Mastery,
	/// Whether what they have is still worth doing. Drains when the world
	/// changes under them, through no fault of theirs.
	Freshness,
}

impl Enumerable for EngagementClass {
	const ALL: &'static [Self] = &[Self::Presence, Self::Momentum, Self::Mastery, Self::Freshness];

	fn index(self) -> usize {
		match self {
			Self::Presence => 0,
			Self::Momentum => 1,
			Self::Mastery => 2,
			Self::Freshness => 3,
		}
	}

	fn discriminant(self) -> u16 {
		match self {
			Self::Presence => 1,
			Self::Momentum => 2,
			Self::Mastery => 3,
			Self::Freshness => 4,
		}
	}

	fn from_discriminant(raw: u16) -> Option<Self> {
		match raw {
			1 => Some(Self::Presence),
			2 => Some(Self::Momentum),
			3 => Some(Self::Mastery),
			4 => Some(Self::Freshness),
			_ => None,
		}
	}
}

impl EngagementClass {
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Presence => "presence",
			Self::Momentum => "momentum",
			Self::Mastery => "mastery",
			Self::Freshness => "freshness",
		}
	}
}

/// Something the system noticed.
///
/// Each variant is a *domain event*, not a clock reading. That is the whole
/// distinction: the engine is driven by these, and time only decays what they
/// deposited.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum StudySignal {
	/// The server prepared a session. Neutral to engagement — it is the
	/// *opportunity*, not the behaviour — but it is what makes a
	/// [`StudyAction::LessonReady`] sayable at all.
	SessionProvisioned { session_id: String },

	/// They sat down. The strongest positive signal there is.
	SessionStarted { session_id: String },

	/// They finished. Restores momentum, and mastery in proportion to how it
	/// went.
	SessionCompleted { session_id: String, score: f64 },

	/// They started and did not finish. Drains momentum specifically, which is
	/// what earns the resume prompt rather than a generic reminder.
	SessionAbandoned { session_id: String, elapsed_ms: i64 },

	/// A graded exercise came back below target. Drains mastery — the case that
	/// should produce *review*, not *more*.
	ScoredBelowTarget { activity_id: String, score: f64 },

	/// New material exists. Drains freshness for everyone it applies to,
	/// without anyone having done anything.
	CurriculumUpdated { curriculum_id: String },

	/// The app changed enough that what they knew is stale.
	AppUpdated { version: String },
}

impl Signal for StudySignal {
	type Class = EngagementClass;

	fn class(&self) -> EngagementClass {
		match self {
			// Provisioning is an opportunity, not engagement. It is filed under
			// freshness with a zero delta so it still lands in the ledger's
			// history without moving the battery.
			Self::SessionProvisioned { .. } | Self::CurriculumUpdated { .. } | Self::AppUpdated { .. } => EngagementClass::Freshness,
			Self::SessionStarted { .. } => EngagementClass::Presence,
			Self::SessionCompleted { .. } | Self::SessionAbandoned { .. } => EngagementClass::Momentum,
			Self::ScoredBelowTarget { .. } => EngagementClass::Mastery,
		}
	}

	fn delta(&self) -> f64 {
		match self {
			Self::SessionProvisioned { .. } => 0.0,
			Self::SessionStarted { .. } => 45.0,
			// A good session restores fully; a poor one still counts for
			// having been finished. Someone who shows up and struggles is more
			// engaged than someone who does not show up.
			Self::SessionCompleted { score, .. } => 45.0_f64.mul_add(score.clamp(0.0, 1.0), 25.0),
			// Abandoning late is worse than abandoning early: it means the
			// session lost them partway rather than never fitting.
			Self::SessionAbandoned { elapsed_ms, .. } => {
				// Clamped to an hour before the cast, so the precision the lint
				// warns about is unreachable: the value is at most 3_600_000.
				#[allow(clippy::cast_precision_loss)]
				let minutes = f64::from(u32::try_from((*elapsed_ms).clamp(0, 3_600_000)).unwrap_or(0)) / 60_000.0;
				-15.0 - minutes
			}
			Self::ScoredBelowTarget { score, .. } => -20.0 * (1.0 - score.clamp(0.0, 1.0)),
			Self::CurriculumUpdated { .. } => -35.0,
			Self::AppUpdated { .. } => -15.0,
		}
	}
}

impl StudySignal {
	/// Stable name for logs and for the wire.
	#[must_use]
	pub const fn kind(&self) -> &'static str {
		match self {
			Self::SessionProvisioned { .. } => "session-provisioned",
			Self::SessionStarted { .. } => "session-started",
			Self::SessionCompleted { .. } => "session-completed",
			Self::SessionAbandoned { .. } => "session-abandoned",
			Self::ScoredBelowTarget { .. } => "scored-below-target",
			Self::CurriculumUpdated { .. } => "curriculum-updated",
			Self::AppUpdated { .. } => "app-updated",
		}
	}
}

/// What the system can say.
///
/// A notification is one *actuation* of one of these, not the thing itself. The
/// beta wires only [`Self::LessonReady`] to Web Push; the rest exist so the
/// selector has somewhere to go, and so adding a channel is not a redesign.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum StudyAction {
	/// The one the beta ships. A session was prepared and they have not sat
	/// down.
	LessonReady { session_id: String },
	/// They stopped partway. Different words, different deep link.
	ResumeAbandoned { session_id: String },
	/// The material is not landing; offer review rather than more.
	SuggestReview { session_id: String },
	/// New material they have not seen.
	NewMaterial { session_id: String },
}

impl StudyAction {
	#[must_use]
	pub const fn kind(&self) -> &'static str {
		match self {
			Self::LessonReady { .. } => "lesson-ready",
			Self::ResumeAbandoned { .. } => "resume-abandoned",
			Self::SuggestReview { .. } => "suggest-review",
			Self::NewMaterial { .. } => "new-material",
		}
	}

	#[must_use]
	pub fn session_id(&self) -> &str {
		match self {
			Self::LessonReady { session_id } | Self::ResumeAbandoned { session_id } | Self::SuggestReview { session_id } | Self::NewMaterial { session_id } => session_id,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{EngagementClass, StudySignal};
	use intervention::{Enumerable, Signal};

	#[test]
	fn all_lists_every_class_and_indexes_are_dense() {
		// `ALL` is the one thing a domain can get wrong without the compiler
		// noticing: a missing variant becomes a class that never decays.
		assert_eq!(EngagementClass::ALL.len(), 4);
		for (position, class) in EngagementClass::ALL.iter().enumerate() {
			assert_eq!(class.index(), position);
		}
	}

	#[test]
	fn discriminants_round_trip_and_are_distinct() {
		let mut seen = Vec::new();
		for class in EngagementClass::ALL {
			let raw = class.discriminant();
			assert!(!seen.contains(&raw), "discriminant {raw} reused");
			seen.push(raw);
			assert_eq!(EngagementClass::from_discriminant(raw), Some(*class));
		}
		assert_eq!(EngagementClass::from_discriminant(9999), None);
	}

	#[test]
	fn setbacks_drain_and_wins_restore() {
		assert!(StudySignal::SessionStarted { session_id: "s".into() }.delta() > 0.0);
		assert!(
			StudySignal::SessionCompleted {
				session_id: "s".into(),
				score: 0.9
			}
			.delta()
				> 0.0
		);
		assert!(
			StudySignal::SessionAbandoned {
				session_id: "s".into(),
				elapsed_ms: 600_000
			}
			.delta()
				< 0.0
		);
		assert!(StudySignal::CurriculumUpdated { curriculum_id: "c".into() }.delta() < 0.0);
	}

	#[test]
	fn showing_up_and_struggling_beats_not_showing_up() {
		let struggled = StudySignal::SessionCompleted {
			session_id: "s".into(),
			score: 0.0,
		};
		let abandoned = StudySignal::SessionAbandoned {
			session_id: "s".into(),
			elapsed_ms: 0,
		};
		assert!(struggled.delta() > abandoned.delta());
	}

	#[test]
	fn abandoning_late_costs_more_than_abandoning_early() {
		let early = StudySignal::SessionAbandoned {
			session_id: "s".into(),
			elapsed_ms: 0,
		};
		let late = StudySignal::SessionAbandoned {
			session_id: "s".into(),
			elapsed_ms: 30 * 60_000,
		};
		assert!(late.delta() < early.delta());
	}

	#[test]
	fn provisioning_is_an_opportunity_not_engagement() {
		let provisioned = StudySignal::SessionProvisioned { session_id: "s".into() };
		assert!((provisioned.delta() - 0.0).abs() < f64::EPSILON);
		assert_eq!(provisioned.class(), EngagementClass::Freshness);
	}

	#[test]
	fn each_setback_drains_the_class_that_earns_its_own_message() {
		assert_eq!(
			StudySignal::SessionAbandoned {
				session_id: "s".into(),
				elapsed_ms: 1
			}
			.class(),
			EngagementClass::Momentum
		);
		assert_eq!(
			StudySignal::ScoredBelowTarget {
				activity_id: "a".into(),
				score: 0.2
			}
			.class(),
			EngagementClass::Mastery
		);
	}
}
