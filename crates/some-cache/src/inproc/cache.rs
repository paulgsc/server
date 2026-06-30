//! `cache` — `InProcCache`, the assembled sharded cache. **Milestones M2.1 + M2.5.**
//!
//! This type ties the pieces together: a fixed array of cache-line-padded
//! [`Shard`]s (M2.2), each holding a CLOCK [`LruMap`] (M2.3), fronted by a
//! [`SingleFlight`] registry (M2.4). It implements [`InProcStore`] (M2.1) and
//! adds the async [`InProcCache::try_get_with`] that `DedupCache` will call at
//! cutover (M2.5), replacing `moka::future::Cache`.
//!
//! ## Shard selection
//!
//! Pick a power-of-two shard count so `hash(key) & (shards - 1)` selects a shard
//! with a mask instead of a `%` (no division on the hot path). The same key-hash
//! flows down into the `LruMap`, so the `String` key is hashed exactly once and
//! never stored.

use crate::inproc::{shard::Shard, single_flight::SingleFlight, InProcStore};
use std::sync::Arc;

/// Sharded, bounded, single-flight in-process cache. The hand-rolled stand-in
/// for `moka::future::Cache<String, Arc<[u8]>>`.
pub struct InProcCache {
	/// One allocation; length is always a power of two.
	shards: Box<[Shard]>,
	/// `shards.len() - 1`, used to map a hash to a shard index with `&`.
	shard_mask: usize,
	/// Coalesces concurrent fetches for the same key.
	single_flight: SingleFlight,
}

impl InProcCache {
	/// Build a cache holding at most `max_entries` live entries total, split
	/// across an internally chosen power-of-two number of shards.
	///
	/// Per-shard capacity is `max_entries / shard_count` (rounded up); justify
	/// your shard count (a small multiple of the core count is typical).
	#[must_use]
	pub fn with_capacity(max_entries: u64) -> Self {
		let _ = max_entries;
		todo!("M2.1/M2.2: choose shard_count (pow2), build Box<[Shard]>, set shard_mask")
	}

	/// Hash `key` and select its shard via the mask.
	#[must_use]
	fn shard_for(&self, key: &str) -> &Shard {
		let _ = (key, &self.shards, self.shard_mask);
		todo!("M2.2: hash key (one pass) → index = hash & shard_mask → &self.shards[index]")
	}

	/// Single-flight fetch: returns the cached bytes if present; otherwise runs
	/// `init` exactly once per key while concurrent callers await the same
	/// result, then caches and returns it.
	///
	/// Replaces `moka::future::Cache::try_get_with`. See
	/// [`crate::inproc::single_flight`] for the cancellation-safety contract you
	/// must uphold — document it in a `# Cancellation` section here when you
	/// implement this (M2.4).
	///
	/// # Errors
	///
	/// Returns the stringified fetch error if `init` fails. (At cutover this
	/// becomes `DedupCacheError`; `String` keeps the skeleton dependency-light.)
	// Skeleton: the body is `todo!()` and awaits nothing yet; the real M2.4
	// implementation awaits the fetch / waiter notification.
	#[allow(clippy::unused_async)]
	pub async fn try_get_with<F, Fut>(&self, key: &str, init: F) -> Result<Arc<[u8]>, String>
	where
		F: FnOnce() -> Fut + Send,
		Fut: std::future::Future<Output = Result<Arc<[u8]>, String>> + Send,
	{
		let _ = (key, init, &self.single_flight);
		todo!("M2.4: fast-path get; else leader/follower coordination via SingleFlight")
	}
}

impl InProcStore for InProcCache {
	fn get(&self, key: &str) -> Option<Arc<[u8]>> {
		let _ = key;
		todo!("M2.3: shard_for(key) → lock → map.get(hash)")
	}

	fn insert(&self, key: &str, value: Arc<[u8]>) {
		let _ = (key, value);
		todo!("M2.3: shard_for(key) → lock → map.insert(hash, value)")
	}

	fn remove(&self, key: &str) -> bool {
		let _ = key;
		todo!("M2.3: shard_for(key) → lock → map.remove(hash)")
	}

	fn invalidate_all(&self) {
		todo!("M2.3: lock each shard in turn → map.clear()")
	}

	fn len(&self) -> usize {
		todo!("M2.3: sum map.len() across shards")
	}
}
