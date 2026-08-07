use crate::engine::{Calibration, Domain, Enumerable, Signal};
use chrono::{DateTime, Duration, Utc};
use std::marker::PhantomData;

/// How far one class has fallen from full, weighted by how much it matters.
///
/// The reason [`Charge`] is a vector: this is what tells a selector *which*
/// intervention fits, where the scalar aggregate only says *whether* one is
/// warranted at all.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Deficit<K> {
	pub class: K,
	/// Current level, after decay.
	pub level: f64,
	/// `(ceiling - level) * weight`. Sorted descending by the caller, so the
	/// first entry is the dominant cause.
	pub shortfall: f64,
}

/// A subject's engagement, one level per class.
///
/// Levels are stored with the instant they were last written and **decayed on
/// read**. Nothing ticks them. A subject who has not been seen in a year costs
/// exactly one exponential when someone finally asks.
#[derive(Debug, Clone)]
pub struct Charge<D: Domain> {
	levels: Vec<f64>,
	as_of: DateTime<Utc>,
	marker: PhantomData<fn() -> D>,
}

/// How far ahead [`Charge::eligible_at`] will look before giving up and asking
/// to be re-checked. Bounds the search and keeps a subject who will not become
/// eligible for months from being written with an absurd timestamp.
const MAX_HORIZON_DAYS: i64 = 30;

/// Iterations of bisection. 2^-40 of a 30-day window is well under a second,
/// which is far finer than any waker interval will ever be.
const BISECTION_STEPS: u32 = 40;

impl<D: Domain> Charge<D> {
	/// A fully charged subject — someone who has just signed up, or just been
	/// seen. Starting full rather than empty matters: an empty charge is
	/// instantly eligible, so a new account would be nudged before it had done
	/// anything.
	#[must_use]
	pub fn full<C: Calibration<D>>(at: DateTime<Utc>) -> Self {
		Self {
			levels: D::Class::ALL.iter().map(|class| C::ceiling(*class)).collect(),
			as_of: at,
			marker: PhantomData,
		}
	}

	/// Rebuild from storage.
	///
	/// `stored` carries `(discriminant, level)` pairs. Unknown discriminants —
	/// classes a previous release knew and this one does not — are skipped
	/// rather than fatal, which is the quarantine arm
	/// [`Enumerable::from_discriminant`] exists for. Classes present in this
	/// build but absent from storage start full, so adding a class in release
	/// N+1 does not make everyone instantly eligible.
	#[must_use]
	pub fn from_storage<C: Calibration<D>>(stored: &[(u16, f64)], as_of: DateTime<Utc>) -> Self {
		let mut charge = Self::full::<C>(as_of);
		for (raw, level) in stored {
			if let Some(class) = D::Class::from_discriminant(*raw) {
				charge.levels[class.index()] = level.clamp(0.0, C::ceiling(class));
			}
		}
		charge.as_of = as_of;
		charge
	}

	/// `(discriminant, level)` pairs for persistence, plus the instant they
	/// are as of. Levels are stored undecayed — decay is a function of the
	/// stamp, and writing decayed values on every read would be the polling
	/// this design exists to avoid.
	#[must_use]
	pub fn to_storage(&self) -> (Vec<(u16, f64)>, DateTime<Utc>) {
		let rows = D::Class::ALL.iter().map(|class| (class.discriminant(), self.levels[class.index()])).collect();
		(rows, self.as_of)
	}

	#[must_use]
	pub const fn as_of(&self) -> DateTime<Utc> {
		self.as_of
	}

	/// One class's level at `at`, after exponential decay.
	#[must_use]
	pub fn level_at<C: Calibration<D>>(&self, class: D::Class, at: DateTime<Utc>) -> f64 {
		decay(self.levels[class.index()], self.as_of, at, C::half_life(class))
	}

	/// The scalar the threshold is compared against.
	///
	/// A positively-weighted sum of decaying terms, so it is strictly
	/// decreasing between signals — which is exactly the property
	/// [`Self::eligible_at`] relies on.
	#[must_use]
	pub fn aggregate<C: Calibration<D>>(&self, at: DateTime<Utc>) -> f64 {
		D::Class::ALL.iter().map(|class| C::weight(*class) * self.level_at::<C>(*class, at)).sum()
	}

	/// Fold in a signal, decaying to `at` first so the delta lands on a current
	/// level rather than a stale one.
	pub fn apply<C: Calibration<D>>(&mut self, signal: &D::Signal, at: DateTime<Utc>) {
		self.settle::<C>(at);
		let class = signal.class();
		let level = &mut self.levels[class.index()];
		*level = (*level + signal.delta()).clamp(0.0, C::ceiling(class));
	}

	/// Credit every class with what an intervention buys back.
	pub fn recharge<C: Calibration<D>>(&mut self, at: DateTime<Utc>) {
		self.settle::<C>(at);
		for class in D::Class::ALL {
			let level = &mut self.levels[class.index()];
			*level = (*level + C::recharge_on_intervention(*class)).clamp(0.0, C::ceiling(*class));
		}
	}

	/// Collapse decay into the stored levels and move the stamp forward.
	fn settle<C: Calibration<D>>(&mut self, at: DateTime<Utc>) {
		for class in D::Class::ALL {
			self.levels[class.index()] = self.level_at::<C>(*class, at);
		}
		self.as_of = at;
	}

	/// Per-class shortfall at `at`, dominant cause first.
	#[must_use]
	pub fn deficits<C: Calibration<D>>(&self, at: DateTime<Utc>) -> Vec<Deficit<D::Class>> {
		let mut deficits: Vec<Deficit<D::Class>> = D::Class::ALL
			.iter()
			.map(|class| {
				let level = self.level_at::<C>(*class, at);
				Deficit {
					class: *class,
					level,
					shortfall: (C::ceiling(*class) - level) * C::weight(*class),
				}
			})
			.collect();

		deficits.sort_by(|a, b| b.shortfall.partial_cmp(&a.shortfall).unwrap_or(std::cmp::Ordering::Equal));
		deficits
	}

	/// **The instant this subject becomes eligible.**
	///
	/// This is the whole scheduling mechanism, and it is arithmetic rather than
	/// a timer. The aggregate is a positively-weighted sum of decaying
	/// exponentials, so between signals it is strictly decreasing and crosses
	/// the threshold exactly once. Bisection finds that crossing; the caller
	/// stores it; the waker selects on it.
	///
	/// A single class would have a closed form
	/// (`t = as_of + half_life · log₂(level/θ)`), but classes have different
	/// half-lives and a sum of exponentials with distinct rates does not invert
	/// in elementary functions. Forty bisection steps over a bounded window
	/// resolve it to well under a second — far finer than any waker cadence —
	/// and cost a few dozen multiplications, once, per signal.
	///
	/// Returns `at` when already eligible, and the horizon when the subject
	/// stays engaged past it: a re-check, never a missed one.
	#[must_use]
	pub fn eligible_at<C: Calibration<D>>(&self, at: DateTime<Utc>) -> DateTime<Utc> {
		if self.aggregate::<C>(at) <= C::THRESHOLD {
			return at;
		}

		let horizon = at + Duration::days(MAX_HORIZON_DAYS);
		if self.aggregate::<C>(horizon) > C::THRESHOLD {
			return horizon;
		}

		let (mut low, mut high) = (at, horizon);
		for _ in 0..BISECTION_STEPS {
			let span = high - low;
			let mid = low + span / 2;
			if mid == low || mid == high {
				break;
			}
			if self.aggregate::<C>(mid) > C::THRESHOLD {
				low = mid;
			} else {
				high = mid;
			}
		}

		// The upper bound: the first instant known to be at or below the
		// threshold. Erring the other way would wake a subject who is not yet
		// eligible, and the waker would have nothing to do.
		high
	}
}

/// Exponential decay by half-life. `level · 2^(-Δt / half_life)`.
fn decay(level: f64, from: DateTime<Utc>, to: DateTime<Utc>, half_life: Duration) -> f64 {
	let elapsed = to - from;
	if elapsed <= Duration::zero() {
		return level;
	}

	let half_life_secs = half_life.num_milliseconds();
	if half_life_secs <= 0 {
		return 0.0;
	}

	#[allow(clippy::cast_precision_loss)]
	let periods = elapsed.num_milliseconds() as f64 / half_life_secs as f64;
	level * 0.5_f64.powf(periods)
}

#[cfg(test)]
mod tests {
	use super::{Charge, Deficit};
	use crate::engine::{Calibration, Domain, Enumerable, Signal};
	use chrono::{DateTime, Duration, TimeZone as _, Utc};

	#[derive(Debug, Clone, Copy, PartialEq, Eq)]
	enum Class {
		Fast,
		Slow,
	}

	impl Enumerable for Class {
		const ALL: &'static [Self] = &[Self::Fast, Self::Slow];

		fn index(self) -> usize {
			match self {
				Self::Fast => 0,
				Self::Slow => 1,
			}
		}

		fn discriminant(self) -> u16 {
			match self {
				Self::Fast => 1,
				Self::Slow => 2,
			}
		}

		fn from_discriminant(raw: u16) -> Option<Self> {
			match raw {
				1 => Some(Self::Fast),
				2 => Some(Self::Slow),
				_ => None,
			}
		}
	}

	struct Nudge(Class, f64);

	impl Signal for Nudge {
		type Class = Class;

		fn class(&self) -> Class {
			self.0
		}

		fn delta(&self) -> f64 {
			self.1
		}
	}

	struct TestDomain;

	impl Domain for TestDomain {
		type Class = Class;
		type Signal = Nudge;
		type Action = &'static str;
	}

	struct Cal;

	impl Calibration<TestDomain> for Cal {
		fn weight(_: Class) -> f64 {
			1.0
		}

		fn half_life(class: Class) -> Duration {
			match class {
				Class::Fast => Duration::days(1),
				Class::Slow => Duration::days(8),
			}
		}

		fn ceiling(_: Class) -> f64 {
			100.0
		}

		fn recharge_on_intervention(_: Class) -> f64 {
			40.0
		}

		const THRESHOLD: f64 = 50.0;
		const REFRACTORY: Duration = Duration::hours(6);
	}

	fn t0() -> DateTime<Utc> {
		Utc.with_ymd_and_hms(2026, 8, 5, 12, 0, 0).unwrap()
	}

	#[test]
	fn a_new_subject_starts_full_rather_than_eligible() {
		let charge = Charge::<TestDomain>::full::<Cal>(t0());
		assert!((charge.aggregate::<Cal>(t0()) - 200.0).abs() < 1e-9);
		assert!(charge.eligible_at::<Cal>(t0()) > t0(), "a fresh account must not be instantly nudgeable");
	}

	#[test]
	fn inactivity_alone_depletes_it_no_signals_required() {
		let charge = Charge::<TestDomain>::full::<Cal>(t0());
		let later = t0() + Duration::days(4);
		assert!(charge.aggregate::<Cal>(later) < charge.aggregate::<Cal>(t0()));
	}

	#[test]
	fn decay_is_computed_not_ticked() {
		// Ten years on: one exponential, not 3652 iterations.
		let charge = Charge::<TestDomain>::full::<Cal>(t0());
		let far = t0() + Duration::days(3652);
		assert!(charge.aggregate::<Cal>(far) < 1e-6);
	}

	#[test]
	fn the_crossing_instant_is_solved_and_lands_on_the_threshold() {
		let charge = Charge::<TestDomain>::full::<Cal>(t0());
		let eligible = charge.eligible_at::<Cal>(t0());

		assert!(charge.aggregate::<Cal>(eligible) <= Cal::THRESHOLD);
		// And it is the *first* such instant: a second earlier is still above.
		assert!(charge.aggregate::<Cal>(eligible - Duration::seconds(1)) > Cal::THRESHOLD);
	}

	#[test]
	fn an_already_depleted_subject_is_eligible_now_rather_than_in_the_past() {
		let mut charge = Charge::<TestDomain>::full::<Cal>(t0());
		charge.apply::<Cal>(&Nudge(Class::Fast, -100.0), t0());
		charge.apply::<Cal>(&Nudge(Class::Slow, -100.0), t0());
		assert_eq!(charge.eligible_at::<Cal>(t0()), t0());
	}

	#[test]
	fn a_drain_brings_the_crossing_forward_and_a_win_pushes_it_back() {
		let baseline = Charge::<TestDomain>::full::<Cal>(t0()).eligible_at::<Cal>(t0());

		let mut drained = Charge::<TestDomain>::full::<Cal>(t0());
		drained.apply::<Cal>(&Nudge(Class::Fast, -60.0), t0());
		assert!(drained.eligible_at::<Cal>(t0()) < baseline);

		// Already full, so a win cannot bank immunity — the ceiling holds.
		let mut topped = Charge::<TestDomain>::full::<Cal>(t0());
		topped.apply::<Cal>(&Nudge(Class::Fast, 500.0), t0());
		assert_eq!(topped.eligible_at::<Cal>(t0()), baseline);
	}

	#[test]
	fn intervening_recharges_so_the_next_pass_does_not_refire() {
		let mut charge = Charge::<TestDomain>::full::<Cal>(t0());
		charge.apply::<Cal>(&Nudge(Class::Fast, -100.0), t0());
		charge.apply::<Cal>(&Nudge(Class::Slow, -100.0), t0());
		assert_eq!(charge.eligible_at::<Cal>(t0()), t0());

		charge.recharge::<Cal>(t0());
		assert!(charge.eligible_at::<Cal>(t0()) > t0(), "an intervention must buy time or the waker loops");
	}

	#[test]
	fn deficits_name_the_dominant_cause_not_just_the_total() {
		let mut charge = Charge::<TestDomain>::full::<Cal>(t0());
		charge.apply::<Cal>(&Nudge(Class::Slow, -80.0), t0());

		let deficits: Vec<Deficit<Class>> = charge.deficits::<Cal>(t0());
		assert_eq!(deficits[0].class, Class::Slow);
		assert!(deficits[0].shortfall > deficits[1].shortfall);
	}

	#[test]
	fn storage_round_trips_and_quarantines_a_class_this_build_does_not_know() {
		let mut charge = Charge::<TestDomain>::full::<Cal>(t0());
		charge.apply::<Cal>(&Nudge(Class::Fast, -30.0), t0());
		let (rows, as_of) = charge.to_storage();

		let restored = Charge::<TestDomain>::from_storage::<Cal>(&rows, as_of);
		assert!((restored.aggregate::<Cal>(t0()) - charge.aggregate::<Cal>(t0())).abs() < 1e-9);

		// A discriminant from a release that knew more than this one.
		let with_stranger = Charge::<TestDomain>::from_storage::<Cal>(&[(1, 10.0), (999, 5.0)], t0());
		assert!((with_stranger.level_at::<Cal>(Class::Fast, t0()) - 10.0).abs() < 1e-9);
		// And a class absent from storage starts full rather than at zero.
		assert!((with_stranger.level_at::<Cal>(Class::Slow, t0()) - 100.0).abs() < 1e-9);
	}

	#[test]
	fn the_horizon_bounds_a_subject_who_stays_engaged() {
		let charge = Charge::<TestDomain>::full::<Cal>(t0());
		let eligible = charge.eligible_at::<Cal>(t0());
		assert!(eligible <= t0() + Duration::days(super::MAX_HORIZON_DAYS));
	}
}
