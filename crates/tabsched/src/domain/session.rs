use crate::domain::ids::{ResourceId, SlotIndex, TrackId};

/// A session is the atomic unit of the system — exactly one per slot.
///
/// `track` and `resource` identify *what was executed*. `slot_index`
/// is the position in the global history (0-based, monotonic).
///
/// # Outcome
///
/// `outcome` is **write-only metadata**. The scheduler never reads it.
/// Storing it here (rather than in a separate log) keeps the data model
/// flat and avoids a parallel structure that must be kept in sync, but
/// the invariant that it does not influence `next_session` is enforced
/// at the `core` layer — not here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
	pub slot_index: SlotIndex,
	pub track: TrackId,
	pub resource: ResourceId,
	pub outcome: Outcome,
}

/// Observed outcome of a session.
///
/// None of these variants affect scheduling. They exist solely for
/// post-hoc analysis and human reflection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Outcome {
	#[default]
	Unrecorded,
	Progress,
	Stuck,
	Review,
}
