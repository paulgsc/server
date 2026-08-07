//! What the study app currently knows how to notice, and what it can say back.
//!
//! This crate is **expected to churn**. Every user story adds a variant here;
//! none of them should touch `intervention`. That split is the point: the
//! structure of "accumulate signals, decide, act" is stable, and the vocabulary
//! of lessons and sessions is not.
//!
//! ## The release boundary
//!
//! [`StudyV1`] is one point in the sample space `intervention` describes. A
//! deployed binary picks it (`type Deployed = StudyV1;`) and thereby closes its
//! world: matches are exhaustive, the calibration table is a total function the
//! compiler checks, and nothing reaches for a string or a `dyn`. Release N+1
//! adds `StudyV2` beside it rather than editing it in place, so the two can
//! coexist while stored charges migrate.
//!
//! Discriminants on [`EngagementClass`] are the durable half of that boundary
//! and are **append-only**. Reusing one for a different meaning silently
//! reinterprets every stored row.
//!
//! ## Why these four classes
//!
//! Not "one battery" but four, because the aggregate decides *whether* to
//! intervene and the individual deficits decide *what to say*:
//!
//! - [`EngagementClass::Presence`] — are they turning up at all? Fast half-life;
//!   this is the one plain absence drains.
//! - [`EngagementClass::Momentum`] — do they finish what they start? Abandonment
//!   drains it specifically, which is what lets an abandoned session produce a
//!   *resume* prompt rather than a generic reminder.
//! - [`EngagementClass::Mastery`] — is the material landing? Poor scores drain
//!   it; this is the one that asks for review rather than more.
//! - [`EngagementClass::Freshness`] — is what they have still worth doing? New
//!   curriculum drains it *without the person doing anything wrong*, which is
//!   the case a pure activity model cannot express at all.

pub mod calibration;
pub mod signal;

pub use calibration::{StudyCalibration, StudySelector};
pub use signal::{EngagementClass, StudyAction, StudySignal};

use intervention::Domain;

/// The vocabulary shipped by the current release.
#[derive(Debug, Clone, Copy)]
pub struct StudyV1;

impl Domain for StudyV1 {
	type Class = EngagementClass;
	type Signal = StudySignal;
	type Action = StudyAction;
}
