use std::collections::HashMap;

use crate::domain::ids::{ResourceId, TrackId};
use crate::domain::topology::Topology;

/// Per-leaf position in the round-robin resource cycle.
///
/// `CursorMap` is a pure value type. Advancing a cursor returns a new
/// `CursorMap`; the original is unchanged. This keeps `next_session` /
/// `apply` free of in-place mutation and therefore easy to test.
///
/// # Invariant
///
/// Every leaf track in `topology` has a corresponding entry. This is
/// established by `CursorMap::new` and maintained by `advance`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorMap(HashMap<TrackId, u64>);

impl CursorMap {
	/// Create a zeroed cursor map for all leaf tracks in `topology`.
	pub fn new(topology: &Topology) -> Self {
		let inner = topology.leaf_tracks().map(|t| (t.id(), 0u64)).collect();
		Self(inner)
	}

	/// Current resource for `track_id` given its resource slice.
	///
	/// Returns `None` only if `track_id` is not a leaf (i.e. absent from
	/// the map) — callers in `select` guarantee this never happens.
	pub fn current_resource(&self, track_id: TrackId, resources: &[ResourceId]) -> Option<ResourceId> {
		let pos = self.0.get(&track_id)?;
		let idx = (*pos as usize) % resources.len();
		Some(resources[idx])
	}

	/// Return a new `CursorMap` with `track_id`'s position incremented.
	///
	/// Panics if `track_id` is not present (logic error in caller).
	pub fn advance(&self, track_id: TrackId) -> Self {
		let mut inner = self.0.clone();
		*inner.get_mut(&track_id).expect("leaf track not in cursor map") += 1;
		Self(inner)
	}

	/// Raw cursor value — useful for persistence/serialization.
	pub fn get(&self, track_id: TrackId) -> Option<u64> {
		self.0.get(&track_id).copied()
	}
}
