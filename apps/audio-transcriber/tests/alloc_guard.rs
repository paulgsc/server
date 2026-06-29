/// Zero-allocation test harness for the hotpath module.
///
/// This module provides a custom global allocator that panics on any allocation
/// when the guard is active. Wrap any code block with `assert_no_alloc!` to
/// verify it makes zero heap allocations.
///
/// # Usage
///
/// ```rust
/// use alloc_guard::assert_no_alloc;
///
/// assert_no_alloc!({
///     let mut buf = RealTimeAudioBuffer::<48_000>::new();
///     buf.push_frame(&[0.0; 160]).unwrap();
/// });
/// ```
///
/// If the block allocates, the test will panic with a message indicating where
/// the allocation occurred.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, Ordering};

/// Global flag: when true, any allocation panics.
static ALLOC_FORBIDDEN: AtomicBool = AtomicBool::new(false);
/// Counts allocations (even when not forbidden, for informational use).
static ALLOC_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub struct GuardedAllocator;

unsafe impl GlobalAlloc for GuardedAllocator {
	unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
		if ALLOC_FORBIDDEN.load(Ordering::SeqCst) {
			panic!(
				"ZERO-ALLOC VIOLATION: heap allocation of {} bytes (align {}) \
                 occurred inside an assert_no_alloc block. \
                 The hotpath must not allocate.",
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
		if ALLOC_FORBIDDEN.load(Ordering::SeqCst) {
			panic!(
				"ZERO-ALLOC VIOLATION: heap reallocation from {} to {} bytes \
                 occurred inside an assert_no_alloc block.",
				layout.size(),
				new_size
			);
		}
		ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
		System.realloc(ptr, layout, new_size)
	}
}

#[global_allocator]
static GLOBAL: GuardedAllocator = GuardedAllocator;

/// Enable the allocation guard. Any allocation after this call panics.
///
/// Must be paired with `disable_alloc_guard()`.
/// Prefer using the `assert_no_alloc!` macro which handles pairing automatically.
pub fn enable_alloc_guard() {
	ALLOC_FORBIDDEN.store(true, Ordering::SeqCst);
}

/// Disable the allocation guard.
pub fn disable_alloc_guard() {
	ALLOC_FORBIDDEN.store(false, Ordering::SeqCst);
}

/// Run `$block` with the allocation guard active.
/// If any allocation occurs, the test panics with a descriptive message.
///
/// # Example
///
/// ```rust
/// assert_no_alloc!({
///     buf.push_frame(&frame).unwrap();
/// });
/// ```
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

/// Return the total number of allocations since process start.
/// Useful for before/after comparison when assert_no_alloc is too strict.
pub fn alloc_count() -> usize {
	ALLOC_COUNT.load(Ordering::Relaxed)
}

/// Run `$block` and return the number of allocations it made.
#[macro_export]
macro_rules! count_allocs {
	($block:block) => {{
		let before = $crate::alloc_count();
		let result = $block;
		let after = $crate::alloc_count();
		(result, after - before)
	}};
}
