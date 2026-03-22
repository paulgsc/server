use std::collections::HashMap;

use crate::domain::ids::TrackId;
use crate::domain::session::Session;

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
