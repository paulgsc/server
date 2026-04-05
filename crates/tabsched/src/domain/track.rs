use std::num::NonZeroU32;

use crate::domain::ids::{ResourceId, TrackId};

/// A node in the scheduling tree.
///
/// # Invariants
///
/// - Exactly one of:
///   - Internal → has children
///   - Leaf     → has resources
/// - `base_target > 0`
/// - Internal → children is non-empty
/// - Leaf     → resources is non-empty
///
/// These are enforced at construction time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
	id: TrackId,
	parent: Option<TrackId>,
	label: String,
	base_target: NonZeroU32,
	kind: TrackKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackKind {
	Internal { children: Vec<TrackId> },
	Leaf { resources: Vec<ResourceId> },
}

impl Track {
	// ── Constructors ───────────────────────────────────────────────────────

	pub fn internal(id: TrackId, parent: Option<TrackId>, label: impl Into<String>, base_target: NonZeroU32, children: Vec<TrackId>) -> Result<Self, TrackError> {
		if children.is_empty() {
			return Err(TrackError::InternalWithNoChildren);
		}

		Ok(Self {
			id,
			parent,
			label: label.into(),
			base_target,
			kind: TrackKind::Internal { children },
		})
	}

	pub fn leaf(id: TrackId, parent: Option<TrackId>, label: impl Into<String>, base_target: NonZeroU32, resources: Vec<ResourceId>) -> Result<Self, TrackError> {
		if resources.is_empty() {
			return Err(TrackError::LeafWithNoResources);
		}

		Ok(Self {
			id,
			parent,
			label: label.into(),
			base_target,
			kind: TrackKind::Leaf { resources },
		})
	}

	// ── Accessors ─────────────────────────────────────────────────────────

	pub fn id(&self) -> TrackId {
		self.id
	}

	pub fn parent(&self) -> Option<TrackId> {
		self.parent
	}

	pub fn label(&self) -> &str {
		&self.label
	}

	pub fn base_target(&self) -> u32 {
		self.base_target.get()
	}

	pub fn is_leaf(&self) -> bool {
		matches!(self.kind, TrackKind::Leaf { .. })
	}

	pub fn is_internal(&self) -> bool {
		matches!(self.kind, TrackKind::Internal { .. })
	}

	pub fn children(&self) -> &[TrackId] {
		match &self.kind {
			TrackKind::Internal { children } => children,
			TrackKind::Leaf { .. } => &[],
		}
	}

	pub fn resources(&self) -> &[ResourceId] {
		match &self.kind {
			TrackKind::Leaf { resources } => resources,
			TrackKind::Internal { .. } => &[],
		}
	}

	pub fn kind(&self) -> &TrackKind {
		&self.kind
	}
}

// ── Errors ────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum TrackError {
	#[error("internal track must have at least one child")]
	InternalWithNoChildren,
	#[error("leaf track must have at least one resource")]
	LeafWithNoResources,
}
