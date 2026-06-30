//! `shard` — one lock-protected, cache-line-padded slice of the cache.
//! **Milestone M2.2.**
//!
//! ## The mechanical-sympathy core: false sharing
//!
//! A single global `Mutex<Map>` serializes every cache operation across every
//! core — the whole reason concurrent caches shard. But sharding naively
//! reintroduces a subtler problem: if two shards' `Mutex` words land in the same
//! 64-byte cache line, two cores hammering two *different* shards still fight
//! over one line via the cache-coherence protocol (MESI). The locks are
//! logically independent but physically adjacent — **false sharing**. Throughput
//! collapses and it looks like lock contention that isn't there.
//!
//! The fix is to pad each shard to its own cache line so independent shards
//! never share one. That is what `#[repr(align(64))]` buys here.
//!
//! ## Your task
//!
//! - Confirm/justify the 64-byte alignment (it is the common x86-64 / aarch64 line size; be ready
//!   to explain why padding to it matters and why over- padding wastes memory).
//! - Decide whether `std::sync::Mutex` is the right primitive or whether a `parking_lot`-style lock
//!   would help — and resist adding a dependency to find out. (`std::sync::Mutex` is now quite
//!   good; measure before reaching.)
//!
//! ## The bar
//!
//! `tests/milestones.rs::milestone_m2_2` asserts `align_of::<Shard>() >= 64`,
//! and the benchmark in `docs/ROADMAP.md` should show the padded layout beating
//! the unpadded one under multi-core contention. If it doesn't, you haven't
//! demonstrated the lesson — investigate before declaring M2.2 done.

use crate::inproc::lru::LruMap;
use std::sync::Mutex;

/// One cache shard, aligned to a full cache line so that locks in adjacent
/// shards never land on the same line (false-sharing avoidance).
#[repr(align(64))]
pub struct Shard {
	map: Mutex<LruMap>,
}

impl Shard {
	/// Build a shard whose inner map holds at most `capacity` entries.
	#[must_use]
	pub fn new(capacity: usize) -> Self {
		Self {
			map: Mutex::new(LruMap::new(capacity)),
		}
	}

	/// Access the inner map under the shard lock.
	///
	/// Returns the guard rather than taking a closure so call sites stay flat;
	/// keep the critical section short — never `.await` while holding it.
	pub fn lock(&self) -> std::sync::MutexGuard<'_, LruMap> {
		todo!("M2.2: lock the mutex, recovering from poisoning as the crate decides")
	}
}
