use super::window::WindowCounter;
use crate::domain::{ids::TrackId, topology::Topology};

/// Compute the scheduling deficit for a single track.
///
/// `deficit(T) = effective_target(T) - sessions(T, W)`
///
/// A positive deficit means the track is under-served; the selection
/// algorithm picks the child with the largest deficit at each level.
///
/// `effective_target` may differ from `base_target` if an adaptation
/// layer is active (see `Adjustment` below). When no adaptation is
/// configured, `adjustment = 0`.
pub fn deficit<W: WindowCounter>(track_id: TrackId, topology: &Topology, window: &W, adjustment: i32) -> i32 {
	let track = topology.track(track_id).expect("deficit called with unknown track");
	let target = track.base_target() as i32 + adjustment;
	target - window.count(track_id) as i32
}

/// Select the child of `parent` with the highest deficit.
///
/// Tie-breaking: lowest `TrackId` value wins (deterministic ordering).
///
/// # Panics
///
/// Panics if `parent` is a leaf (no children). Callers in `select`
/// guarantee this is never reached.
pub fn argmax_deficit<W: WindowCounter>(parent: TrackId, topology: &Topology, window: &W, adjustments: &dyn Fn(TrackId) -> i32) -> TrackId {
	let track = topology.track(parent).expect("unknown track");
	let children = track.children();
	debug_assert!(!children.is_empty(), "argmax_deficit called on leaf");

	children
		.iter()
		.copied()
		.max_by(|&a, &b| {
			let da = deficit(a, topology, window, adjustments(a));
			let db = deficit(b, topology, window, adjustments(b));
			da.cmp(&db).then(b.cmp(&a)) // tie: lower id wins (reverse)
		})
		.expect("children is non-empty")
}

// ── Adjustment ─────────────────────────────────────────────────────────────

/// Bounded, slow-changing offset to `base_target`.
///
/// From the spec:
///   |adjustment| ≤ α * base_target
///
/// The `Adjustments` map is not part of the scheduling core — it is an
/// optional overlay. When absent, every track has `adjustment = 0`.
#[derive(Debug, Clone, Default)]
pub struct Adjustments {
	inner: std::collections::HashMap<TrackId, i32>,
}

impl Adjustments {
	pub fn set(&mut self, track: TrackId, topology: &Topology, adj: i32, alpha: f32) {
		let base = topology.track(track).map(|t| t.base_target() as f32).unwrap_or(0.0);
		let max = (alpha * base).round() as i32;
		self.inner.insert(track, adj.clamp(-max, max));
	}

	pub fn get(&self, track: TrackId) -> i32 {
		self.inner.get(&track).copied().unwrap_or(0)
	}
}
