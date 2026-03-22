use super::{
	cursor::CursorMap,
	deficit::{argmax_deficit, Adjustments},
	window::WindowCounter,
};
use crate::domain::{
	ids::{ResourceId, TrackId},
	topology::Topology,
};

/// Result of one selection pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
	pub leaf: TrackId,
	pub resource: ResourceId,
}

/// Determine the next `(leaf, resource)` pair without mutating any state.
///
/// # Algorithm
///
/// 1. Start at the topology root.
/// 2. While the current node has children: pick the child with the
///    largest effective deficit (ties broken by lowest `TrackId`).
/// 3. At the leaf: read the current cursor position to get the resource.
///
/// This function is deliberately `fn`, not a method, to make the purity
/// contract explicit at the call-site. All inputs are borrowed; no state
/// is changed.
pub fn next_selection<W: WindowCounter>(topology: &Topology, cursors: &CursorMap, window: &W, adjustments: &Adjustments) -> Selection {
	let adj_fn = |id: TrackId| adjustments.get(id);

	// --- hierarchical descent ---
	let mut current = topology.root();
	loop {
		let track = topology.track(current).expect("selection encountered unknown track");

		if track.is_leaf() {
			break;
		}

		current = argmax_deficit(current, topology, window, &adj_fn);
	}

	// --- resource selection ---
	let leaf = current;
	let track = topology.track(leaf).expect("leaf track not found");
	let resource = cursors.current_resource(leaf, track.resources()).expect("cursor not found for leaf");

	Selection { leaf, resource }
}
