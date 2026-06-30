//! Zero-allocation test harness for the `inproc` cache + the zero-alloc half of
//! the milestone spec.
//!
//! This file installs a custom global allocator that panics on any heap
//! allocation while the guard is active. Wrap a block in `assert_no_alloc!` to
//! prove it touches the heap zero times. It is the mechanical-sympathy
//! equivalent of a unit test: a function can be "correct" and still be a
//! performance bug if it allocates on a path that runs on every cache hit.
//!
//! The milestone tests below are `#[ignore]`d while the implementation is
//! `todo!()`. As you land each milestone, delete its `#[ignore]` and make it
//! green. A milestone is not done until its tests here AND in `milestones.rs`
//! pass with the `inproc` lint contract intact.
//!
//! Run the zero-alloc spec:  `cargo test -p some-cache --test alloc_guard -- --ignored`

use std::{
	alloc::{GlobalAlloc, Layout, System},
	sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

/// When true, any allocation panics.
static ALLOC_FORBIDDEN: AtomicBool = AtomicBool::new(false);
/// Counts allocations (informational, even when not forbidden).
static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

pub struct GuardedAllocator;

unsafe impl GlobalAlloc for GuardedAllocator {
	unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
		// `swap(false)` disarms the guard *before* we panic — the panic machinery
		// itself allocates, and re-entering an armed guard would double-panic and
		// SIGABRT instead of producing a catchable failure. The paired
		// `disable_alloc_guard()` in `assert_no_alloc!` is then a harmless no-op.
		if ALLOC_FORBIDDEN.swap(false, Ordering::SeqCst) {
			panic!(
				"ZERO-ALLOC VIOLATION: heap allocation of {} bytes (align {}) inside an \
				 assert_no_alloc block. The cache hot path must not allocate.",
				layout.size(),
				layout.align()
			);
		}
		ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
		System.alloc(layout)
	}

	unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
		System.dealloc(ptr, layout)
	}

	unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
		if ALLOC_FORBIDDEN.swap(false, Ordering::SeqCst) {
			panic!(
				"ZERO-ALLOC VIOLATION: heap reallocation from {} to {new_size} bytes inside an assert_no_alloc block.",
				layout.size()
			);
		}
		ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
		System.realloc(ptr, layout, new_size)
	}
}

#[global_allocator]
static GLOBAL: GuardedAllocator = GuardedAllocator;

/// Enable the guard. Any allocation after this call panics. Pair with `disable`.
pub fn enable_alloc_guard() {
	ALLOC_FORBIDDEN.store(true, Ordering::SeqCst);
}

/// Disable the guard.
pub fn disable_alloc_guard() {
	ALLOC_FORBIDDEN.store(false, Ordering::SeqCst);
}

/// Total allocations since process start.
pub fn alloc_count() -> usize {
	ALLOC_COUNT.load(Ordering::Relaxed)
}

/// Run `$block` with the allocation guard active; panic if it allocates.
#[macro_export]
macro_rules! assert_no_alloc {
	($block:block) => {{
		$crate::enable_alloc_guard();
		let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| $block));
		$crate::disable_alloc_guard();
		match result {
			Ok(v) => v,
			Err(e) => std::panic::resume_unwind(e),
		}
	}};
}

// ============================================================================
// Harness self-test — proves the guard actually fires. Safe to run today.
//
// IMPORTANT: the guard is a *process-global* flag. Tests that use
// `assert_no_alloc!` must never run concurrently with each other, or one test
// will flip the flag while another is mid-block. That is why the self-checks
// live in a single sequential test, and why the `#[ignore]`d zero-alloc
// milestone tests below must be run with `-- --test-threads=1`:
//
//   cargo test -p some-cache --test alloc_guard -- --ignored --test-threads=1
// ============================================================================

#[test]
fn harness_self_test() {
	// 1. A stack-only block must pass cleanly.
	assert_no_alloc!({
		let mut acc = 0_u64;
		for i in 0..1000 {
			acc = acc.wrapping_add(i);
		}
		std::hint::black_box(acc);
	});

	// 2. A heap allocation must trip the guard (panic caught here).
	let caught = std::panic::catch_unwind(|| {
		assert_no_alloc!({
			let v: Vec<u8> = Vec::with_capacity(64);
			std::hint::black_box(&v);
		});
	});
	assert!(caught.is_err(), "guard must panic on a heap allocation");
}

// ============================================================================
// M2.0 — `rng::probability_gate` must not allocate (runs on every cache hit)
// ============================================================================

#[test]
#[ignore = "M2.0: implement rng::probability_gate, then delete this #[ignore]"]
fn milestone_m2_0_probability_gate_zero_alloc() {
	use some_cache::inproc::rng::probability_gate;
	// Warm the thread-local PRNG once (its lazy seed may allocate) before guarding.
	let _ = probability_gate(0.5);
	assert_no_alloc!({
		let mut hits = 0_u32;
		for _ in 0..10_000 {
			if probability_gate(0.3) {
				hits += 1;
			}
		}
		std::hint::black_box(hits);
	});
}

// ============================================================================
// M2.3 — warm `LruMap::get`/`insert` must not allocate
// ============================================================================

#[test]
#[ignore = "M2.3: implement LruMap, then delete this #[ignore]"]
fn milestone_m2_3_warm_get_insert_zero_alloc() {
	use some_cache::inproc::lru::LruMap;
	use std::sync::Arc;

	let mut map = LruMap::new(16);
	let v: Arc<[u8]> = Arc::from(&b"payload"[..]);

	// Fill to capacity so steady-state insert reuses slots (eviction), never grows.
	for k in 0..16_u64 {
		map.insert(k, Arc::clone(&v));
	}

	assert_no_alloc!({
		let _ = map.get(3);
		// Inserting a fresh key at capacity must evict-in-place, not allocate.
		map.insert(99, Arc::clone(&v));
		std::hint::black_box(map.get(99));
	});
}
