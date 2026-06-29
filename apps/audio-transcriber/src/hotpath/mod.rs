
/// Hotpath module — real-time audio processing boundary.
///
/// # Invariants
///
/// Every submodule in this tree must uphold:
/// - Zero dynamic allocations in the steady-state execution path
/// - No blocking system calls (Mutex, file I/O, network, thread::sleep)
/// - No async functions or .await calls
/// - No tracing macros in execution loops
/// - All loops bounded by compile-time constants
///
/// See `hotpath/README.md` for the full contract.
/// See `docs/adr/001-hotpath-isolation.md` for the rationale.
//
// Lint configuration for this module:
// We allow a small set of pedantic lints that conflict with real-time patterns.
// Everything else is deny.
#![allow(clippy::inline_always)] // we explicitly want force-inline on critical path

pub mod buffer;
pub mod knn;
pub mod spsc;

// gpu module is gated — only available when CUDA feature is enabled
#[cfg(feature = "cuda")]
pub mod gpu;

// Re-export the public API surface
pub use buffer::RealTimeAudioBuffer;
pub use knn::{FixedKnnEngine, FixedNeighborSet, Neighbor};
pub use spsc::SpscRingBuffer;

/// Marker trait: types that are safe to use inside the real-time hotpath.
///
/// A type implements `HotpathSafe` if and only if:
/// - All of its methods are allocation-free in steady state
/// - None of its methods block on a system primitive
///
/// This trait has no methods — it is purely a documentation and
/// type-system marker. It cannot be auto-derived; implementors
/// must consciously attest to the invariants.
///
/// # Safety
///
/// Implementing this trait incorrectly (e.g., on a type that allocates)
/// does not cause memory unsafety, but will cause real-time constraint
/// violations at runtime.
pub unsafe trait HotpathSafe {}

// Safety: RealTimeAudioBuffer is stack-allocated with a fixed CAP.
// push_frame and drain are allocation-free. See ADR-002.
unsafe impl<const CAP: usize> HotpathSafe for RealTimeAudioBuffer<CAP> {}

// Safety: SpscRingBuffer is statically bounded. try_push and try_pop
// are non-blocking and allocation-free. See ADR-003.
unsafe impl<T: Send, const N: usize> HotpathSafe for SpscRingBuffer<T, N> {}
