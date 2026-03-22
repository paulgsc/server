/// Opaque identifier for a [`Track`](super::track::Track).
///
/// Newtypes are used throughout instead of raw integers so that
/// `TrackId` and `ResourceId` are never accidentally interchanged at
/// call-sites. Both are `Copy` — they are indices, not owned handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TrackId(pub u32);

/// Opaque identifier for a [`Resource`](super::resource::Resource).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceId(pub u32);

/// Monotonically increasing index of a [`Session`](super::session::Session)
/// within the history. Slot 0 is the first session ever executed.
///
/// Using a dedicated newtype prevents confusing a slot index with a
/// track/resource id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SlotIndex(pub u64);

impl SlotIndex {
	pub fn as_u64(self) -> u64 {
		self.0
	}

	pub fn next(self) -> Self {
		Self(self.0 + 1)
	}
}

impl std::fmt::Display for TrackId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "T{}", self.0)
	}
}

impl std::fmt::Display for ResourceId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "R{}", self.0)
	}
}

impl std::fmt::Display for SlotIndex {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "#{}", self.0)
	}
}
