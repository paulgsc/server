//! Time, as the limiter reads it: monotonic, in nanoseconds, and injected.

use std::{
	sync::atomic::{AtomicU64, Ordering},
	time::{Duration, Instant},
};

/// Nanoseconds since this clock's own origin.
///
/// Must never go backwards. Only differences between two readings mean
/// anything, so the origin is arbitrary.
pub trait Clock: Send + Sync {
	fn now(&self) -> u64;
}

/// The production clock: [`Instant`], which the OS guarantees monotonic.
pub struct MonotonicClock {
	origin: Instant,
}

impl MonotonicClock {
	#[must_use]
	pub fn new() -> Self {
		Self { origin: Instant::now() }
	}
}

impl Default for MonotonicClock {
	fn default() -> Self {
		Self::new()
	}
}

impl Clock for MonotonicClock {
	fn now(&self) -> u64 {
		u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX)
	}
}

/// A clock that moves only when told to, for tests that need exact time.
#[derive(Default)]
pub struct ManualClock {
	now: AtomicU64,
}

impl ManualClock {
	#[must_use]
	pub const fn new() -> Self {
		Self { now: AtomicU64::new(0) }
	}

	pub fn advance(&self, by: Duration) {
		let by = u64::try_from(by.as_nanos()).unwrap_or(u64::MAX);
		self.now.fetch_add(by, Ordering::AcqRel);
	}
}

impl Clock for ManualClock {
	fn now(&self) -> u64 {
		self.now.load(Ordering::Acquire)
	}
}

impl<C: Clock + ?Sized> Clock for std::sync::Arc<C> {
	fn now(&self) -> u64 {
		(**self).now()
	}
}
