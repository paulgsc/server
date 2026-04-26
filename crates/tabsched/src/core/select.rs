use super::{
	cursor::CursorMap,
	deficit::{argmax_score, Adjustments, RecencyMap},
	window::{SubtreeWindow, WindowCounter},
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
///    highest *combined score*:
///
///    ```text
///    score(T) = clamped_deficit(T) − recency_penalty(T)
///    clamped_deficit(T) = max(0, effective_target(T) − subtree_count(T, W))
///    ```
///
///    Ties are broken by lowest `TrackId` (deterministic).
/// 3. At the leaf: read the current cursor position to get the resource.
///
/// # Why `SubtreeWindow`
///
/// The raw `WindowCounter` (e.g. `RingWindow`) records only the **leaf**
/// `TrackId` selected each slot.  When `argmax_score` compares internal
/// nodes at a non-leaf level, `window.count(internal_id)` would always
/// return 0, giving every internal node a deficit equal to its full
/// `base_target`.  In that case the only distinguishing criterion is
/// `TrackId` tie-breaking, which permanently locks the scheduler into
/// whichever subtree has the lowest-numbered internal node — even if that
/// subtree is far over-budget.
///
/// [`SubtreeWindow`] fixes this by making `count(internal_id)` return the
/// sum of leaf counts within that subtree, so internal-node deficits
/// correctly reflect actual usage.
///
/// This function is deliberately `fn`, not a method, to make the purity
/// contract explicit at the call-site. All inputs are borrowed; no state
/// is changed.
pub fn next_selection<W: WindowCounter>(topology: &Topology, cursors: &CursorMap, window: &W, adjustments: &Adjustments, recency: &RecencyMap) -> Selection {
	// Wrap the raw window so that internal-node counts aggregate their
	// subtree's leaf sessions.  This is the primary fix for branch-level
	// starvation: without this, every internal node always returns count=0
	// and selection degenerates to TrackId tie-breaking at every non-leaf level.
	let subtree_window = SubtreeWindow::new(window, topology);

	let adj_fn = |id: TrackId| adjustments.get(id);

	// --- hierarchical descent ---
	let mut current = topology.root();
	loop {
		let track = topology.track(current).expect("selection encountered unknown track");

		if track.is_leaf() {
			break;
		}

		current = argmax_score(current, topology, &subtree_window, &adj_fn, recency);
	}

	// --- resource selection ---
	let leaf = current;
	let track = topology.track(leaf).expect("leaf track not found");
	let resource = cursors.current_resource(leaf, track.resources()).expect("cursor not found for leaf");

	Selection { leaf, resource }
}
