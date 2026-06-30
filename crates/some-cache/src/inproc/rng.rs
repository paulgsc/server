//! `rng` — a tiny, dependency-free probability gate. **Milestone M2.0 (warmup).**
//!
//! ## The dependency we are removing
//!
//! `crate::store::CacheStore::should_touch` calls `fastrand::f64() < p` to decide
//! whether a cache hit refreshes its TTL. That is the *only* use of `fastrand`
//! in the crate — an entire dependency for one biased coin flip.
//!
//! ## Your task
//!
//! Implement [`probability_gate`] backed by a thread-local xorshift64* (or PCG,
//! or `SplitMix64` — your choice, defend it). Then change `should_touch` to call
//! `crate::inproc::rng::probability_gate(p)` and delete `fastrand` from
//! `Cargo.toml`.
//!
//! ## Why this is the warmup (Dreyfus: Novice → Advanced Beginner)
//!
//! It is small and self-contained, but it surfaces real concepts you will lean
//! on in the harder milestones:
//!   - **Interior mutability without locks**: a `thread_local!` `Cell<u64>` lets each thread mutate
//!     its own PRNG state through a shared `&` — no `&mut`, no `Mutex`. Be ready to explain *why
//!     that is sound* (no aliasing across threads; `Cell` is `!Sync` and never crosses a thread
//!     boundary).
//!   - **Float construction from bits**: turning a `u64` into a uniform `f64` in `[0, 1)` is a
//!     bit-layout exercise, not a division. Look up the 53-bit mantissa trick before you reach for
//!     `as f64 / u64::MAX as f64`.
//!
//! ## The bar
//!
//! `probability_gate` must make **zero heap allocations** (it runs on every
//! cache hit) — `tests/milestones.rs::milestone_m2_0` asserts this under the
//! `alloc_guard` harness. `p <= 0.0` must always be `false`; `p >= 1.0` must
//! always be `true`; values in between must be statistically uniform.

use std::cell::Cell;

thread_local! {
	/// Per-thread PRNG state. Seeded lazily on first use.
	static STATE: Cell<u64> = Cell::new(seed());
}

/// Produce a non-zero seed for this thread's PRNG.
///
/// xorshift breaks if seeded with 0 — mixing a per-thread address with a
/// monotonic counter is a cheap way to avoid that and to decorrelate threads.
fn seed() -> u64 {
	todo!("M2.0: derive a non-zero, per-thread seed (address + counter, then mix)")
}

/// Advance the thread-local PRNG and return the next 64-bit value.
fn next_u64() -> u64 {
	todo!("M2.0: xorshift64* step over the thread-local STATE Cell")
}

/// Return `true` with probability `p`, clamped to `[0.0, 1.0]`.
///
/// `p <= 0.0` ⇒ always `false`; `p >= 1.0` ⇒ always `true`. Must not allocate.
#[must_use]
pub fn probability_gate(p: f64) -> bool {
	if p <= 0.0 {
		return false;
	}
	if p >= 1.0 {
		return true;
	}
	let _ = next_u64;
	todo!("M2.0: build a uniform f64 in [0,1) from next_u64() bits and compare to p")
}
