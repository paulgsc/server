//! One GCRA state per client, in a [`DashMap`].
//!
//! The key is whatever the caller partitions by (`file_host` uses the peer's
//! `PeerKey`). It is a partition key, never a metric label: the store forgets
//! a client once its bucket is full again, so it is bounded by how many
//! clients sent something within the last `burst × interval`, while a label
//! on a counter would live forever.

use super::{
	clock::{Clock, MonotonicClock},
	gcra::{self, Decision, Quota},
};
use dashmap::{mapref::entry::Entry, DashMap};
use std::hash::Hash;

pub struct KeyedLimiter<K, C = MonotonicClock> {
	quota: Quota,
	clock: C,
	tats: DashMap<K, u64>,
}

/// What the limiter holds right now, read without changing it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snapshot {
	/// Clients whose bucket is not full.
	pub tracked: usize,
	/// Mean requests admissible right now across tracked clients; the full
	/// burst when none are tracked, which is what a first request would see.
	pub average_available: f64,
}

impl<K: Eq + Hash + Clone, C: Clock> KeyedLimiter<K, C> {
	#[must_use]
	pub fn new(quota: Quota, clock: C) -> Self {
		Self {
			quota,
			clock,
			tats: DashMap::new(),
		}
	}

	#[must_use]
	pub const fn quota(&self) -> &Quota {
		&self.quota
	}

	/// Decides one request from `key`, and records it if admitted.
	///
	/// The read and the write are one step under the key's shard lock, so two
	/// requests from one client cannot both spend the same allowance, and a
	/// busy client never sees a refusal it didn't earn.
	pub fn check(&self, key: &K) -> Decision {
		let now = self.clock.now();
		match self.tats.entry(key.clone()) {
			Entry::Occupied(mut held) => {
				let (decision, next) = gcra::decide(Some(*held.get()), now, &self.quota);
				if let Some(next) = next {
					*held.get_mut() = next;
				}
				decision
			}
			Entry::Vacant(empty) => {
				let (decision, next) = gcra::decide(None, now, &self.quota);
				if let Some(next) = next {
					empty.insert(next);
				}
				decision
			}
		}
	}

	/// Forgets every client whose bucket is full again, returning how many.
	///
	/// Changes no decision: a forgotten client's next request is decided
	/// exactly as a full bucket's would be.
	pub fn evict_full(&self) -> usize {
		let now = self.clock.now();
		let before = self.tats.len();
		self.tats.retain(|_, tat| !gcra::is_full(*tat, now));
		before.saturating_sub(self.tats.len())
	}

	/// Reads the limiter's state without changing it.
	#[must_use]
	pub fn snapshot(&self) -> Snapshot {
		let now = self.clock.now();
		let (mut tracked, mut available) = (0usize, 0u64);
		for entry in &self.tats {
			if !gcra::is_full(*entry.value(), now) {
				tracked += 1;
				available += u64::from(gcra::available(*entry.value(), now, &self.quota));
			}
		}
		#[allow(clippy::cast_precision_loss)] // counts of clients and requests, far below 2^52
		let average_available = if tracked == 0 {
			f64::from(self.quota.burst())
		} else {
			available as f64 / tracked as f64
		};
		Snapshot { tracked, average_available }
	}
}

#[cfg(test)]
mod tests {
	//! Every test here runs on a [`ManualClock`]: exact time, no sleeps. The
	//! old limiter's tests slept once and read once, which is the one pattern
	//! its bug could not show in; these drive whole schedules, with the
	//! production quota and the production 10 s sweep interleaved.

	use super::*;
	use crate::rate_limiter::ManualClock;
	use std::{num::NonZeroU32, sync::Arc, time::Duration};

	const MS: Duration = Duration::from_millis(1);
	const SWEEP: Duration = Duration::from_secs(10);

	/// `file_host`'s quota: `RATE_LIMIT`'s default, 60 a minute.
	fn production() -> Quota {
		Quota::per_period(NonZeroU32::new(60).unwrap(), Duration::from_secs(60))
	}

	fn limiter(quota: Quota) -> (KeyedLimiter<&'static str, Arc<ManualClock>>, Arc<ManualClock>) {
		let clock = Arc::new(ManualClock::new());
		(KeyedLimiter::new(quota, Arc::clone(&clock)), clock)
	}

	/// A small deterministic generator, so a failure names its seed.
	struct Rng(u64);

	impl Rng {
		fn below(&mut self, bound: u64) -> u64 {
			self.0 ^= self.0 << 13;
			self.0 ^= self.0 >> 7;
			self.0 ^= self.0 << 17;
			self.0 % bound
		}
	}

	/// One step of a schedule: wait, then send `requests` at once.
	#[derive(Clone, Copy)]
	struct Step {
		wait: Duration,
		requests: u32,
	}

	/// Runs `steps` for one client, also running the periodic sweep (evict,
	/// then snapshot, as `file_host`'s `metrics::periodic` does) every `sweep`
	/// if given. Returns every decision in order.
	fn run(quota: Quota, steps: &[Step], sweep: Option<Duration>) -> Vec<Decision> {
		let (limiter, clock) = limiter(quota);
		let mut decisions = Vec::new();
		let mut until_sweep = sweep;
		for step in steps {
			let mut wait = step.wait;
			while let Some(left) = until_sweep.filter(|left| *left <= wait) {
				clock.advance(left);
				wait -= left;
				let _ = limiter.evict_full();
				let _ = limiter.snapshot();
				until_sweep = sweep;
			}
			clock.advance(wait);
			until_sweep = until_sweep.map(|left| left.saturating_sub(wait));
			decisions.extend((0..step.requests).map(|_| limiter.check(&"phone")));
		}
		decisions
	}

	fn refusals(decisions: &[Decision]) -> usize {
		decisions.iter().filter(|decision| !decision.is_allowed()).count()
	}

	/// Steady sending at `gap` for `minutes`, plus `burst` extra requests every
	/// `every`, the first at 700 ms.
	fn steady_with_bursts(gap: Duration, minutes: u64, burst: u32, every: Duration) -> Vec<Step> {
		let end = Duration::from_secs(minutes * 60);
		let mut events: Vec<(Duration, u32)> = std::iter::successors(Some(Duration::ZERO), |at| Some(*at + gap))
			.take_while(|at| *at < end)
			.map(|at| (at, 1))
			.collect();
		if burst > 0 {
			events.extend(
				std::iter::successors(Some(700 * MS), |at| Some(*at + every))
					.take_while(|at| *at < end)
					.map(|at| (at, burst)),
			);
		}
		events.sort_by_key(|(at, _)| *at);
		let mut last = Duration::ZERO;
		events
			.into_iter()
			.map(|(at, requests)| {
				let step = Step {
					wait: at.saturating_sub(last),
					requests,
				};
				last = at;
				step
			})
			.collect()
	}

	/// The promise: a client that stays within its quota is never refused,
	/// whatever shape its traffic has and however often the limiter is read.
	///
	/// "Within its quota" is checked by construction, not by the limiter:
	/// steady sending no faster than one per interval, the shapes a phone
	/// actually sends (the night of 2026-11-07: steady polling plus a burst on
	/// every screen), and random bursts each followed by enough rest to earn
	/// them back. Under the old limiter the second family alone was refused
	/// within minutes.
	#[test]
	fn a_client_under_its_limit_is_never_refused() {
		let quota = production();
		for gap_ms in [1000, 1001, 1200, 1500, 1999, 2500, 60_000] {
			let steps = steady_with_bursts(gap_ms * MS, 30, 0, Duration::ZERO);
			assert_eq!(refusals(&run(quota, &steps, Some(SWEEP))), 0, "steady every {gap_ms} ms");
		}
		// 50 a minute steady, plus 20 at once every two minutes: 60 a minute
		// on average, with the bucket never below 40.
		let steps = steady_with_bursts(1200 * MS, 30, 20, Duration::from_secs(120));
		assert_eq!(refusals(&run(quota, &steps, Some(SWEEP))), 0, "steady 1.2 s plus 20 every 2 min");
		// 40 a minute steady, plus 10 at once every minute.
		let steps = steady_with_bursts(1500 * MS, 30, 10, Duration::from_secs(60));
		assert_eq!(refusals(&run(quota, &steps, Some(SWEEP))), 0, "steady 1.5 s plus 10 every minute");

		for seed in 1..=200 {
			let mut rng = Rng(seed);
			let steps: Vec<Step> = (0..100)
				.flat_map(|_| {
					let burst = 1 + u32::try_from(rng.below(60)).unwrap();
					let rest = quota.interval() * burst + MS * u32::try_from(rng.below(5000)).unwrap();
					[
						Step {
							wait: Duration::ZERO,
							requests: burst,
						},
						Step { wait: rest, requests: 0 },
					]
				})
				.collect();
			let sweep = Duration::from_millis(1 + rng.below(20_000));
			assert_eq!(refusals(&run(quota, &steps, Some(sweep))), 0, "seed {seed}");
		}
	}

	/// Observing the limiter must not change what it decides. The old one
	/// refilled on every read and rounded each time, so the 10 s gauge sweep
	/// alone changed which requests were refused. Here any traffic, over the
	/// limit or not, is decided identically with and without reads between
	/// its requests.
	#[test]
	fn reading_the_limiter_never_changes_a_decision() {
		for seed in 1..=300 {
			let mut rng = Rng(seed);
			let quota = Quota::per_period(
				NonZeroU32::new(1 + u32::try_from(rng.below(100)).unwrap()).unwrap(),
				Duration::from_millis(1 + rng.below(120_000)),
			);
			let steps: Vec<Step> = (0..200)
				.map(|_| Step {
					wait: Duration::from_micros(rng.below(4_000_000)),
					requests: u32::try_from(rng.below(8)).unwrap(),
				})
				.collect();
			let unread = run(quota, &steps, None);
			let read = run(quota, &steps, Some(Duration::from_micros(1 + rng.below(3_000_000))));
			assert_eq!(unread, read, "seed {seed}");
		}
	}

	/// The limiter agrees, request for request, with a token bucket written
	/// the obvious way in exact integer arithmetic: credit in nanoseconds,
	/// capped at `burst` intervals, each request costing one interval.
	#[test]
	fn decides_exactly_as_an_exact_token_bucket_would() {
		for seed in 1..=300 {
			let mut rng = Rng(seed);
			let quota = Quota::per_period(
				NonZeroU32::new(1 + u32::try_from(rng.below(100)).unwrap()).unwrap(),
				Duration::from_millis(1 + rng.below(120_000)),
			);
			let cost = quota.interval().as_nanos();
			let cap = cost * u128::from(quota.burst());
			let (mut credit, mut at) = (cap, 0u128);
			let (limiter, clock) = limiter(quota);
			for _ in 0..500 {
				let wait = Duration::from_micros(rng.below(3_000_000));
				clock.advance(wait);
				let now = at + wait.as_nanos();
				credit = (credit + (now - at)).min(cap);
				at = now;
				for _ in 0..rng.below(6) {
					let expected = credit >= cost;
					if expected {
						credit -= cost;
					}
					assert_eq!(limiter.check(&"client").is_allowed(), expected, "seed {seed}");
				}
			}
		}
	}

	#[test]
	fn a_refused_client_is_told_exactly_when_to_return() {
		let (limiter, clock) = limiter(production());
		for left in (0..60).rev() {
			assert_eq!(limiter.check(&"phone"), Decision::Allowed { remaining: left });
		}
		assert_eq!(
			limiter.check(&"phone"),
			Decision::Limited {
				retry_after: Duration::from_secs(1)
			}
		);

		clock.advance(400 * MS);
		assert_eq!(
			limiter.check(&"phone"),
			Decision::Limited { retry_after: 600 * MS },
			"a refusal does not push the return further out"
		);
		clock.advance((600 * MS).saturating_sub(Duration::from_nanos(1)));
		assert!(!limiter.check(&"phone").is_allowed());
		clock.advance(Duration::from_nanos(1));
		assert_eq!(limiter.check(&"phone"), Decision::Allowed { remaining: 0 });
	}

	/// A quota whose arithmetic would overflow refuses rather than wraps into
	/// admitting everything.
	#[test]
	fn a_quota_too_large_to_count_fails_closed() {
		let quota = Quota::per_period(NonZeroU32::new(1).unwrap(), Duration::MAX);
		let (limiter, _clock) = limiter(quota);
		assert!(limiter.check(&"client").is_allowed(), "its one request");
		assert!(!limiter.check(&"client").is_allowed(), "saturating would have admitted this");
		assert!(!limiter.check(&"client").is_allowed());
	}

	/// A full bucket earns nothing: an hour idle buys the burst, not more.
	#[test]
	fn time_spent_full_is_not_credited() {
		let (limiter, clock) = limiter(production());
		clock.advance(Duration::from_secs(3600));
		assert_eq!(limiter.check(&"phone"), Decision::Allowed { remaining: 59 });
		clock.advance(500 * MS);
		assert_eq!(limiter.check(&"phone"), Decision::Allowed { remaining: 58 });
	}

	#[test]
	fn one_client_spending_its_allowance_leaves_another_untouched() {
		let quota = Quota::per_period(NonZeroU32::new(2).unwrap(), Duration::from_secs(60));
		let (limiter, _clock) = limiter(quota);
		assert!(limiter.check(&"a").is_allowed());
		assert!(limiter.check(&"a").is_allowed());
		assert!(!limiter.check(&"a").is_allowed());
		assert!(limiter.check(&"b").is_allowed());
		assert!(limiter.check(&"b").is_allowed());
	}

	/// The store holds only clients that are not back to full, so churning
	/// client identities cannot grow it without bound.
	#[test]
	fn eviction_forgets_only_full_buckets() {
		let clock = Arc::new(ManualClock::new());
		let limiter: KeyedLimiter<u32, _> = KeyedLimiter::new(production(), Arc::clone(&clock));
		for client in 0..1000 {
			assert!(limiter.check(&client).is_allowed());
		}
		assert_eq!(limiter.snapshot().tracked, 1000);
		clock.advance(Duration::from_secs(1).saturating_sub(Duration::from_nanos(1)));
		assert_eq!(limiter.evict_full(), 0, "each is still one request short of full");
		clock.advance(Duration::from_nanos(1));
		assert_eq!(limiter.evict_full(), 1000);
		assert_eq!(
			limiter.snapshot(),
			Snapshot {
				tracked: 0,
				average_available: 60.0
			}
		);
	}

	#[test]
	fn the_snapshot_reports_what_a_request_would_see() {
		let (limiter, clock) = limiter(production());
		assert_eq!(
			limiter.snapshot(),
			Snapshot {
				tracked: 0,
				average_available: 60.0
			}
		);
		for _ in 0..30 {
			let _ = limiter.check(&"a");
		}
		let _ = limiter.check(&"b");
		assert_eq!(
			limiter.snapshot(),
			Snapshot {
				tracked: 2,
				average_available: 44.5
			}
		);
		clock.advance(Duration::from_secs(10));
		assert_eq!(
			limiter.snapshot(),
			Snapshot {
				tracked: 1,
				average_available: 40.0
			},
			"b is full again"
		);
	}

	/// Many threads at one instant admit exactly the burst: none lost to a
	/// race, none refused for contention.
	#[test]
	fn contention_admits_exactly_the_burst() {
		let (limiter, _clock) = limiter(production());
		let admitted = std::sync::atomic::AtomicU32::new(0);
		std::thread::scope(|scope| {
			for _ in 0..8 {
				scope.spawn(|| {
					for _ in 0..50 {
						if limiter.check(&"phone").is_allowed() {
							admitted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
						}
					}
				});
			}
		});
		assert_eq!(admitted.into_inner(), 60);
	}
}
