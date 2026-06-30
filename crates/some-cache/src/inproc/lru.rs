//! `lru` — bounded, type-erased entry map with CLOCK eviction. **Milestone M2.3.**
//!
//! This is the per-shard storage that lives behind one `Mutex`. It is *not*
//! thread-safe on its own — [`crate::inproc::shard::Shard`] owns the lock. Keep
//! it that way: a data structure that assumes single-threaded access can be far
//! simpler and more cache-friendly than one that synchronizes internally.
//!
//! ## Why not a classic doubly-linked-list LRU?
//!
//! The textbook LRU (`HashMap` + intrusive doubly-linked list) is a *cache-hostile*
//! data structure: every `get` chases three or four pointers to unlink and
//! relink a node, each almost certainly a cache miss, and each node is its own
//! heap allocation. That is the opposite of mechanical sympathy.
//!
//! ## What to build instead: CLOCK (second-chance)
//!
//! Store entries in a flat `Box<[Slot]>` (one allocation, contiguous, prefetch
//! friendly) plus a `HashMap<u64, usize>` from key-hash to slot index. Give each
//! slot a `referenced` bit. On `get`, set the bit. On `insert` when full, sweep a
//! "hand" forward: clear set bits, evict the first slot whose bit is already
//! clear. This approximates LRU at O(1) amortized with sequential memory access.
//!
//! Keys are pre-hashed to `u64` by the shard layer so this map never owns a
//! `String` — another allocation removed from the hot path.
//!
//! ## The bar
//!
//! - `len()` never exceeds `capacity`.
//! - `get`/`insert` on a warm map perform **zero heap allocations** (slots and the index map are
//!   sized at construction). `tests/milestones.rs` enforces this under `alloc_guard`.
//! - Eviction is deterministic given an access sequence (so it is testable).

use std::{collections::HashMap, sync::Arc};

/// One CLOCK slot. `referenced` is the second-chance bit.
struct Slot {
	key_hash: u64,
	value: Arc<[u8]>,
	referenced: bool,
	occupied: bool,
}

/// Single-threaded, bounded, CLOCK-evicting map of `u64 -> Arc<[u8]>`.
pub struct LruMap {
	capacity: usize,
	/// Flat slot array — one allocation, scanned sequentially by the clock hand.
	slots: Box<[Slot]>,
	/// key-hash → slot index.
	index: HashMap<u64, usize>,
	/// The clock hand position for the next eviction sweep.
	hand: usize,
}

impl LruMap {
	/// Allocate a map sized for `capacity` entries. All slot/index storage is
	/// reserved here so steady-state `get`/`insert` never allocate.
	#[must_use]
	pub fn new(capacity: usize) -> Self {
		let _ = capacity;
		todo!("M2.3: allocate `capacity` empty slots + a HashMap with that capacity")
	}

	/// Look up `key_hash`, marking the slot referenced (second chance) on hit.
	#[must_use]
	pub fn get(&mut self, key_hash: u64) -> Option<Arc<[u8]>> {
		let _ = key_hash;
		todo!("M2.3: index lookup → set slot.referenced = true → clone out Arc::clone(&slot.value)")
	}

	/// Insert or overwrite. If full, run the CLOCK hand to evict one victim
	/// before claiming its slot.
	pub fn insert(&mut self, key_hash: u64, value: Arc<[u8]>) {
		let _ = (key_hash, value);
		todo!("M2.3: overwrite-if-present; else find/evict a slot via the clock hand and install")
	}

	/// Remove a single key. Returns `true` if it was present.
	#[must_use]
	pub fn remove(&mut self, key_hash: u64) -> bool {
		let _ = key_hash;
		todo!("M2.3: free the slot (occupied=false) and drop the index entry")
	}

	/// Drop all entries; keep the backing allocations for reuse.
	pub fn clear(&mut self) {
		todo!("M2.3: mark every slot unoccupied, clear the index, reset the hand")
	}

	/// Number of occupied slots.
	#[must_use]
	pub fn len(&self) -> usize {
		todo!("M2.3: occupied slot count (== self.index.len())")
	}

	/// Whether the map holds no entries.
	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}
}
