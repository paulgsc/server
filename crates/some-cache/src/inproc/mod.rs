//! `inproc` — the hand-rolled, cache-line-aware, single-flight in-process cache.
//!
//! ## What this module replaces
//!
//! `DedupCache` (see `crate::dedup`) currently leans on
//! `moka::future::Cache<String, Arc<[u8]>>` for exactly three things:
//!
//!   1. **Single-flight** — `try_get_with(key, fut)` runs the init future once per key while
//!      concurrent callers for that key await the same result.
//!   2. **Bounded eviction** — `max_capacity(n)` caps the number of live entries.
//!   3. **Invalidation** — `remove(key)` / `invalidate_all()`.
//!
//! That is a *tiny* slice of moka's surface area. moka itself pulls a transitive
//! tree (`crossbeam-*`, `tagptr`, `quanta`, `event-listener`, …) to deliver a
//! concurrent W-TinyLFU cache we are barely using. This module is the
//! "mechanical sympathy" replacement: a sharded map, padded to cache lines,
//! with a CLOCK eviction approximation and a notify-based single-flight gate —
//! built from `std` + the `tokio` we already depend on, and nothing else.
//!
//! ## The invariant contract (the teeth of the refactor)
//!
//! This subtree carries a deliberately strict lint posture. These denies are
//! not decoration — they fail the build if the implementation drifts back
//! toward the allocation-happy, refcount-churning patterns moka let us ignore.
//! Each milestone (M2.0–M2.5) lands one piece; see `docs/ROADMAP.md`.
//!
//! See `README.md` for the human-readable invariant contract and
//! `docs/adr/ADR-001.md` for why we are doing this at all.

// ── Lint contract ───────────────────────────────────────────────────────────
//
// `restriction`-group lints are off by default; we opt in here, scoped to this
// module only (the rest of the crate does async plumbing where these would be
// noise). Promote to the whole crate only once the cutover (M2.5) is done.
#![deny(clippy::clone_on_ref_ptr)] // Arc/Rc clones must be explicit: `Arc::clone(&x)`, never `x.clone()`.
#![deny(clippy::rc_buffer)] // No `Arc<Vec<T>>` / `Arc<String>`: store `Arc<[u8]>` and skip the double indirection.
#![deny(clippy::mutex_atomic)] // No `Mutex<bool>` / `Mutex<usize>` where an atomic does the job.
#![deny(clippy::mem_forget)] // `mem::forget` on a guard is a leak, not a trick.
#![deny(clippy::trivially_copy_pass_by_ref)] // Pass small `Copy` types by value; don't chase a pointer to read a u64.
#![deny(unsafe_op_in_unsafe_fn)]
// If/when M2.3 goes intrusive, every unsafe op states its invariant.
//
// SKELETON ONLY — remove this allow as the modules below are implemented.
// While bodies are `todo!()`, the fields/types they will use read as dead code.
// The milestone tests (`tests/milestones.rs`) are `#[ignore]`d until you lift
// this and they go green. Deleting this line is itself part of "done".
#![allow(dead_code)]

use std::sync::Arc;

pub mod cache;
pub mod lru;
pub mod rng;
pub mod shard;
pub mod single_flight;

pub use cache::InProcCache;

/// The minimal in-process cache surface `DedupCache` actually needs.
///
/// **M2.1 deliverable.** Defining this trait *before* writing the implementation
/// is the point: it forces you to name the exact slice of moka you depend on,
/// rather than inheriting its entire API by accident. `DedupCache` should be
/// refactored to hold a `C: InProcStore` (or `Arc<dyn InProcStore>`), so the
/// concrete cache is swappable and testable.
///
/// Note the deliberate omission of `try_get_with`: single-flight is `async` and
/// lives as an inherent method on [`InProcCache`] (M2.4) rather than on this
/// trait, to keep the trait object-safe and free of an `async fn in trait`
/// `Send` story you don't need yet. Reaching for that is an Evaluate-level
/// judgement call — be ready to defend it.
pub trait InProcStore {
	/// Fetch a live entry, recording a "use" for the eviction policy.
	fn get(&self, key: &str) -> Option<Arc<[u8]>>;

	/// Insert (or overwrite) an entry, evicting if over capacity.
	fn insert(&self, key: &str, value: Arc<[u8]>);

	/// Drop a single key. Returns `true` if it was present.
	fn remove(&self, key: &str) -> bool;

	/// Drop every entry across every shard.
	fn invalidate_all(&self);

	/// Current live entry count (summed across shards).
	fn len(&self) -> usize;

	/// Convenience derived from [`InProcStore::len`].
	fn is_empty(&self) -> bool {
		self.len() == 0
	}
}
