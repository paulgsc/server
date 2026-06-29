use std::collections::HashMap;

use crate::domain::ids::TrackId;
use crate::domain::session::Session;
use crate::domain::topology::Topology;

/// Count of sessions per leaf track within the rolling window.
///
/// # Why a trait
///
/// Two implementations are provided:
///
/// - [`SlidingWindow`]: O(W) scan over history. Simple, correct,
///   suitable for small W or infrequent queries.
/// - [`RingWindow`]: O(1) updates via ring buffer + per-track counters.
///   Suitable for long-running systems where W is large.
///
/// The trait is `dyn`-free at every call-site — callers are generic over
/// `W: WindowCounter`, which is monomorphised. The trait exists purely
/// to allow swapping the implementation in tests and benchmarks without
/// touching the selection logic.
pub trait WindowCounter {
	/// Number of sessions logged for `track` within the last `window_size`
	/// slots (inclusive of the most recent).
	///
	/// For **leaf** tracks this is the count of sessions on that exact track.
	///
	/// For **internal** tracks callers should use [`SubtreeWindow`] so that
	/// the count aggregates across all leaves in the subtree. The raw
	/// `WindowCounter` implementations only record exact `TrackId` matches,
	/// so calling `count(internal_id)` directly always returns 0 — which
	/// causes the deficit of every internal node to equal its `base_target`,
	/// making tie-breaking by `TrackId` the sole selection criterion at all
	/// non-leaf levels.  That is the primary starvation bug.
	fn count(&self, track: TrackId) -> u32;
}

// ── Sliding window ─────────────────────────────────────────────────────────

/// Exact rolling window over a history slice.
///
/// Computes counts on demand by scanning the tail of `history`.
/// `O(W)` per call, `O(1)` storage.
pub struct SlidingWindow<'a> {
	history: &'a [Session],
	window_size: usize,
}

impl<'a> SlidingWindow<'a> {
	pub fn new(history: &'a [Session], window_size: usize) -> Self {
		Self { history, window_size }
	}
}

impl WindowCounter for SlidingWindow<'_> {
	fn count(&self, track: TrackId) -> u32 {
		let start = self.history.len().saturating_sub(self.window_size);
		self.history[start..].iter().filter(|s| s.track == track).count() as u32
	}
}

// ── Ring-buffer window ─────────────────────────────────────────────────────

/// O(1) window counter backed by a ring buffer and per-track tallies.
///
/// Maintains:
/// - A ring buffer of size `W` recording the `TrackId` of each slot.
/// - A `HashMap<TrackId, u32>` of current counts within the window.
///
/// On each `push`:
/// 1. The slot being evicted decrements its track's count.
/// 2. The new slot increments its track's count.
/// 3. The ring slot is overwritten.
///
/// This is the correct implementation for production use where W may be
/// large (e.g., 50–100 slots).
///
/// # Important: leaf counts only
///
/// `RingWindow` records the exact `TrackId` that was selected (always a
/// leaf).  `count(internal_id)` always returns 0.  Wrap this in a
/// [`SubtreeWindow`] before passing it to [`deficit`] / [`argmax_score`]
/// so that internal nodes are measured by their subtree's session count.
#[derive(Debug, Clone)]
pub struct RingWindow {
	ring: Vec<Option<TrackId>>,
	counts: HashMap<TrackId, u32>,
	head: usize,
	size: usize, // current fill level ≤ capacity
	capacity: usize,
}

impl RingWindow {
	pub fn new(window_size: usize) -> Self {
		assert!(window_size > 0, "window_size must be > 0");
		Self {
			ring: vec![None; window_size],
			counts: HashMap::new(),
			head: 0,
			size: 0,
			capacity: window_size,
		}
	}

	/// Reconstruct a `RingWindow` from an existing history slice.
	/// Useful when restoring from persistence.
	pub fn from_history(history: &[Session], window_size: usize) -> Self {
		let mut w = Self::new(window_size);
		let start = history.len().saturating_sub(window_size);
		for s in &history[start..] {
			w.push(s.track);
		}
		w
	}

	/// Record one new session.
	pub fn push(&mut self, track: TrackId) {
		// evict oldest slot if full
		if self.size == self.capacity {
			if let Some(evicted) = self.ring[self.head] {
				let c = self.counts.get_mut(&evicted).expect("count invariant");
				*c -= 1;
				if *c == 0 {
					self.counts.remove(&evicted);
				}
			}
		} else {
			self.size += 1;
		}
		self.ring[self.head] = Some(track);
		*self.counts.entry(track).or_insert(0) += 1;
		self.head = (self.head + 1) % self.capacity;
	}
}

impl WindowCounter for RingWindow {
	fn count(&self, track: TrackId) -> u32 {
		self.counts.get(&track).copied().unwrap_or(0)
	}
}

// ── Subtree-aggregating window ─────────────────────────────────────────────

/// A [`WindowCounter`] wrapper that makes internal nodes visible.
///
/// # The starvation bug this fixes
///
/// `RingWindow` (and `SlidingWindow`) record the **leaf** `TrackId` that
/// was selected each slot.  When `argmax_score` is called at a non-leaf
/// level it calls `window.count(child_id)` for each child.  For children
/// that are **internal nodes** that returns 0 every time, because internal
/// nodes are never directly "selected" — only their leaves are.
///
/// The consequence is that every internal node always has
///
/// ```text
/// raw_deficit = base_target - 0 = base_target   (maximum possible)
/// ```
///
/// making `base_target` values and `TrackId` tie-breaking the sole
/// selection criteria at every non-leaf level.  Whichever internal-node
/// child has the lowest `TrackId` wins every time, and all branches under
/// higher `TrackId` internal nodes are permanently starved.
///
/// # The fix
///
/// `SubtreeWindow` intercepts `count(id)`:
///
/// - If `id` is a **leaf** in the topology → delegates to the inner
///   counter (exact match, unchanged).
/// - If `id` is an **internal node** → returns the **sum** of `count`
///   over all leaf descendants of that node.
///
/// This makes the deficit of an internal node reflect how many sessions
/// its subtree has actually consumed within the window, which is exactly
/// what the fairness invariant requires.
///
/// # Cost
///
/// `count` for an internal node visits all leaves in the subtree.  In the
/// worst case (root query, depth-1 tree) this is O(L) where L is the
/// number of leaves.  Since `argmax_score` calls this once per sibling at
/// each tree level, total cost per selection is O(L · depth).  For
/// typical topologies (L ≤ 30, depth ≤ 4) this is negligible.
pub struct SubtreeWindow<'a, W> {
	inner: &'a W,
	topology: &'a Topology,
}

impl<'a, W: WindowCounter> SubtreeWindow<'a, W> {
	pub fn new(inner: &'a W, topology: &'a Topology) -> Self {
		Self { inner, topology }
	}

	/// Recursively sum leaf counts under `id`.
	fn subtree_count(&self, id: TrackId) -> u32 {
		match self.topology.track(id) {
			None => 0,
			Some(track) if track.is_leaf() => self.inner.count(id),
			Some(track) => track.children().iter().map(|&c| self.subtree_count(c)).sum(),
		}
	}
}

impl<W: WindowCounter> WindowCounter for SubtreeWindow<'_, W> {
	fn count(&self, track: TrackId) -> u32 {
		self.subtree_count(track)
	}
}
