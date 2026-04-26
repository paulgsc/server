use std::collections::HashMap;

use super::window::WindowCounter;
use crate::domain::{ids::TrackId, topology::Topology};

// ── Deficit ────────────────────────────────────────────────────────────────

/// Compute the **clamped** scheduling deficit for a single track.
///
/// ```text
/// deficit(T) = max(0,  effective_target(T) - sessions(T, W))
/// ```
///
/// # Why clamped?
///
/// The unclamped form (`target - count`) goes deeply negative once a
/// track overshoots its target. A track stuck at, say, `-4` cannot
/// recover until four of its own sessions roll out of the window, even
/// if every other track has deficit 0. That produces long droughts —
/// especially for low-weight tracks whose small target is exceeded after
/// a single extra pick.
///
/// Clamping at 0 removes the "deep negative hole": once a track is at
/// or above its target it simply stops competing on debt grounds; it
/// can still win via recency (see [`score`]).
///
/// `effective_target` may differ from `base_target` when an
/// [`Adjustments`] overlay is active. When absent, `adjustment = 0`.
pub fn deficit<W: WindowCounter>(track_id: TrackId, topology: &Topology, window: &W, adjustment: i32) -> i32 {
	let track = topology.track(track_id).expect("deficit called with unknown track");
	let target = track.base_target() as i32 + adjustment;
	(target - window.count(track_id) as i32).max(0)
}

// ── Recency ────────────────────────────────────────────────────────────────

/// Per-leaf record of how many slots ago each track was last served.
///
/// `RecencyMap` is a pure value type — `record` returns a new map,
/// leaving the original unchanged.
///
/// # Penalty formula
///
/// The recency penalty decays with distance so that a track served
/// just one slot ago is strongly penalised, while one served many
/// slots ago is barely penalised at all:
///
/// ```text
/// penalty(T) = λ / (steps_since_served(T) + 1)
/// ```
///
/// With integer arithmetic (scaled by `LAMBDA_SCALE`) this becomes:
///
/// ```text
/// penalty_i32(T) = LAMBDA_SCALE / (steps_since_served(T) + 1)
/// ```
///
/// The denominator `+1` ensures that a track served *this* slot gets
/// `LAMBDA_SCALE / 1 = LAMBDA_SCALE`, and a track served *last* slot
/// gets `LAMBDA_SCALE / 2`, etc. A track never served gets 0 penalty.
///
/// `LAMBDA_SCALE` is the maximum penalty — chosen so it can suppress a
/// freshly-served track (penalty = LAMBDA_SCALE) but can't suppress a
/// track that has been waiting even one slot (penalty = LAMBDA_SCALE/2).
/// Tune this constant to taste; the default of 4 works well for trees
/// whose `base_target` values are in the 1–10 range.
///
/// # Integer arithmetic
///
/// All arithmetic stays in `i32` to keep the scoring path allocation-free
/// and consistent with [`deficit`]. Fractional precision is not needed;
/// the penalty only needs to create a *relative ordering*, not an
/// exact measure.
#[derive(Debug, Clone, Default)]
pub struct RecencyMap {
	/// `last_served[T]` = slot index when `T` was last selected.
	/// Absent means "never served" → penalty = 0.
	last_served: HashMap<TrackId, u64>,
	/// Monotonically increasing slot counter, incremented in `record`.
	slot: u64,
}

/// Maximum recency penalty (integer, same units as [`deficit`]).
///
/// `penalty(track_served_this_slot) = LAMBDA_SCALE`
/// `penalty(track_served_last_slot) = LAMBDA_SCALE / 2`
/// `penalty(never_served)           = 0`
const LAMBDA_SCALE: i32 = 4;

impl RecencyMap {
	pub fn new() -> Self {
		Self::default()
	}

	/// Reconstruct from history length (slot counter only; per-track
	/// last-served is unavailable without replaying). Used in
	/// `State::from_history` where full replay is performed separately.
	#[allow(dead_code)]
	pub fn from_slot(slot: u64) -> Self {
		Self {
			last_served: HashMap::new(),
			slot,
		}
	}

	/// Record that `track` was served now; advance the slot counter.
	///
	/// Returns a new `RecencyMap`; does not mutate `self`.
	pub fn record(&self, track: TrackId) -> Self {
		let mut next = self.clone();
		next.last_served.insert(track, next.slot);
		next.slot += 1;
		next
	}

	/// Integer recency penalty for `track` at the current slot.
	///
	/// Returns 0 if the track has never been served (no penalty for
	/// tracks that have never competed).
	pub fn penalty(&self, track: TrackId) -> i32 {
		match self.last_served.get(&track) {
			None => 0,
			Some(&last) => {
				let delta = self.slot.saturating_sub(last); // steps since served
				LAMBDA_SCALE / (delta as i32 + 1)
			}
		}
	}
}

// ── Combined score ─────────────────────────────────────────────────────────

/// Combined scheduling score for `track_id`.
///
/// ```text
/// score(T) = clamped_deficit(T) - recency_penalty(T)
/// ```
///
/// A higher score means the track is a better candidate for the next slot.
///
/// - `clamped_deficit` ≥ 0: rewards tracks that are behind on their target.
/// - `recency_penalty` ≥ 0: penalises tracks served recently.
///
/// Together these balance **debt** (long-term proportionality) with
/// **recency** (short-term smoothness), which is the invariant that the
/// windowed deficit alone cannot provide.
pub fn score<W: WindowCounter>(track_id: TrackId, topology: &Topology, window: &W, adjustment: i32, recency: &RecencyMap) -> i32 {
	deficit(track_id, topology, window, adjustment) - recency.penalty(track_id)
}

/// Select the child of `parent` with the highest combined score.
///
/// Tie-breaking: lowest `TrackId` value wins (deterministic ordering).
///
/// # Panics
///
/// Panics if `parent` is a leaf (no children). Callers in `select`
/// guarantee this is never reached.
pub fn argmax_score<W: WindowCounter>(parent: TrackId, topology: &Topology, window: &W, adjustments: &dyn Fn(TrackId) -> i32, recency: &RecencyMap) -> TrackId {
	let track = topology.track(parent).expect("unknown track");
	let children = track.children();
	debug_assert!(!children.is_empty(), "argmax_score called on leaf");

	children
		.iter()
		.copied()
		.max_by(|&a, &b| {
			let sa = score(a, topology, window, adjustments(a), recency);
			let sb = score(b, topology, window, adjustments(b), recency);
			sa.cmp(&sb).then(b.cmp(&a)) // tie: lower id wins (reverse)
		})
		.expect("children is non-empty")
}

// ── Adjustments (unchanged) ────────────────────────────────────────────────

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
