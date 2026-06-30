//! `single_flight` — run an expensive fetch once per key while concurrent
//! callers coalesce onto its result. **Milestone M2.4 (the hard one).**
//!
//! This is the marquee node of the sprint. It is where async, `Send` bounds, and
//! cancellation safety all collide — the concepts the Rust JIT Tutor reserves
//! for Proficient → Expert.
//!
//! ## What `moka::future::Cache::try_get_with` gave us
//!
//! For a given key, the first caller's init future runs; every other caller that
//! arrives while it is in flight *awaits the same result* instead of launching a
//! duplicate fetch. That is the thundering-herd guard `DedupCache` depends on.
//!
//! ## The design you must build
//!
//! A `Mutex<HashMap<u64, Arc<Slot>>>` of in-flight keys. The first caller for a
//! key inserts a `Slot` and becomes the *leader*: it runs the fetch, stores the
//! result in the slot, and wakes waiters. Later callers find the existing slot,
//! become *followers*, and await notification, then read the shared result.
//! `tokio::sync::Notify` is the natural primitive — adding tokio's `sync`
//! feature (we already depend on tokio) is acceptable; adding a new crate is not.
//!
//! ## The trap: cancellation safety  (Tutor scenario — Future cancellation)
//!
//! A `tokio` task can be **dropped at any await point**. If the *leader's* future
//! is cancelled mid-fetch, the slot it registered is still in the map and every
//! follower is parked on a `Notify` that will now never fire — a permanent hang
//! for that key. Rust's type system will **not** catch this; it is an invariant
//! you uphold manually.
//!
//! The fix is a guard whose `Drop` removes the slot and wakes waiters *even on
//! cancellation*, so a follower can be promoted to leader and retry. Before you
//! write code, answer in prose:
//!   1. At which `.await` can the leader be cancelled?
//!   2. What exact state is left behind, and who is now stuck?
//!   3. Why does an RAII `Drop` guard fix it where a trailing cleanup line does not?
//!
//! Document the safety invariant in a `# Cancellation` doc section on the method
//! you add. Skipping that discipline is itself a diagnostic — it means M2.4 isn't
//! actually done.
//!
//! ## The bar
//!
//! `tests/milestones.rs::milestone_m2_4` must show: (a) N concurrent callers for
//! one key trigger exactly **one** fetch; (b) cancelling the leader does not hang
//! the followers.

use std::{collections::HashSet, sync::Mutex};

/// Registry of in-flight fetches, keyed by key-hash.
///
/// Skeleton holds only the key set; M2.4 replaces this with a `HashMap<u64,
/// Arc<Slot>>` whose `Slot` carries the shared result + notification handle.
pub struct SingleFlight {
	in_flight: Mutex<HashSet<u64>>,
}

impl Default for SingleFlight {
	fn default() -> Self {
		Self::new()
	}
}

impl SingleFlight {
	/// Create an empty single-flight registry.
	#[must_use]
	pub fn new() -> Self {
		Self {
			in_flight: Mutex::new(HashSet::new()),
		}
	}

	/// Number of fetches currently in flight (test/observability hook).
	#[must_use]
	pub fn outstanding(&self) -> usize {
		let _ = &self.in_flight;
		todo!("M2.4: length of the in-flight map under its lock")
	}
}
