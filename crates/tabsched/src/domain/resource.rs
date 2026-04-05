use crate::domain::ids::ResourceId;

/// A resource is an opaque handle to something the learner consumes
/// during a session (a tab, a PDF page, a problem set, etc.).
///
/// # Design rationale
///
/// Resources carry **no semantic metadata** (no type tag, no URL, no
/// difficulty) intentionally. The moment `ResourceType` variants exist
/// they will leak into scheduling logic — exactly the failure mode the
/// spec prohibits. Any display/metadata layer lives outside this crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
	pub id: ResourceId,
	/// Human-readable label — used only for display, never for logic.
	pub label: String,
}

impl Resource {
	pub fn new(id: ResourceId, label: impl Into<String>) -> Self {
		Self { id, label: label.into() }
	}
}
