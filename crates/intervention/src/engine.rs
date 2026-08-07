use crate::charge::{Charge, Deficit};
use chrono::{DateTime, Duration, Utc};
use std::marker::PhantomData;

/// A closed set, dense enough to index and stable enough to persist.
///
/// Two numberings on purpose. [`Self::index`] is a dense position used for the
/// in-memory charge vector and may be renumbered freely between releases;
/// [`Self::discriminant`] is written to storage and may never be reused for a
/// different meaning.
pub trait Enumerable: Copy + Eq + Sized + 'static {
	/// Every variant. The compiler checks nothing about this, so it is the one
	/// place a domain can lie to the engine — `ALL` missing a variant shows up
	/// as a class that silently never decays. Domains should derive or test it.
	const ALL: &'static [Self];

	/// Dense position in `ALL`, for vector indexing.
	fn index(self) -> usize;

	/// Stable identifier as written to durable storage.
	fn discriminant(self) -> u16;

	/// Total parse of a stored discriminant.
	///
	/// `None` is the quarantine arm: a row written by a release that knew a
	/// class this build does not. Returning `Option` rather than panicking, and
	/// rather than reaching for `dyn`, is how a closed compile-time world
	/// survives contact with open durable state.
	fn from_discriminant(raw: u16) -> Option<Self>;
}

/// Something that happened, and what it does to engagement.
pub trait Signal {
	type Class: Enumerable;

	fn class(&self) -> Self::Class;

	/// Positive recharges, negative drains.
	///
	/// Expressed as a delta rather than a new level because signals compose:
	/// two poor scores should cost more than one, and neither should overwrite
	/// what a completed session earned.
	fn delta(&self) -> f64;
}

/// The vocabulary of one release.
///
/// Everything product-shaped lives behind this trait, which is what lets the
/// engine be written before the product is designed and survive it being
/// redesigned.
pub trait Domain: 'static {
	type Class: Enumerable + std::fmt::Debug;
	type Signal: Signal<Class = Self::Class>;
	type Action: std::fmt::Debug;
}

/// The numbers. Separated from [`Domain`] because the vocabulary and its tuning
/// change on completely different schedules — a class is a product decision, a
/// half-life is an operational one.
pub trait Calibration<D: Domain> {
	/// How much this class counts toward "engaged". Must be positive, or the
	/// aggregate stops being monotone and [`Charge::eligible_at`] stops being
	/// solvable.
	fn weight(class: D::Class) -> f64;

	/// How long this class's charge takes to halve with no signals at all.
	/// This *is* the inactivity model; there is no separate idle timer.
	fn half_life(class: D::Class) -> Duration;

	/// Full charge. Signals cannot push a class above it, so a burst of wins
	/// cannot bank months of immunity.
	fn ceiling(class: D::Class) -> f64;

	/// What a delivered intervention buys back, per class. Without this a
	/// depleted subject stays eligible and every waker pass re-fires;
	/// [`Calibration::REFRACTORY`] alone would mask that rather than fix it.
	fn recharge_on_intervention(class: D::Class) -> f64;

	/// Aggregate at or below this warrants an intervention.
	const THRESHOLD: f64;

	/// Hard floor between two interventions, whatever the arithmetic says.
	/// Burst protection of last resort.
	const REFRACTORY: Duration;
}

/// May we act, right now?
///
/// Everything a clock-driven design puts in its policy belongs here: quiet
/// hours, presence, per-channel opt-in, "they are mid-session". None of these
/// is a reason to intervene; all of them are reasons not to.
pub trait Admissibility<D: Domain> {
	type Reason: Copy + std::fmt::Debug;

	/// # Errors
	/// The reason it is not admissible, named so a silent system can say why.
	fn admit(&self, at: DateTime<Utc>, action: &D::Action) -> Result<(), Self::Reason>;
}

/// Which intervention, given what is depleted.
///
/// This is why [`Charge`] is a vector and not a scalar. A single number can say
/// *whether* to intervene and can never say *what to say*: "abandoned twenty
/// minutes ago" and "gone for six days" reach the same threshold and want
/// completely different messages.
pub trait Selector<D: Domain> {
	fn select(&self, deficits: &[Deficit<D::Class>]) -> Option<D::Action>;
}

/// What the engine concluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict<A, R> {
	/// Act now.
	Intervene(A),
	/// Not yet. `until` is when it is worth asking again — the solved crossing
	/// instant, or the end of a refractory period. Callers persist it; that is
	/// the whole scheduling mechanism.
	Wait { until: DateTime<Utc> },
	/// Warranted, but not admissible. Distinct from `Wait` because a suppressed
	/// intervention is a fact worth logging, and because the reason is the
	/// first thing anyone asks when a system goes quiet.
	Suppressed { reason: R, retry_at: DateTime<Utc> },
	/// Warranted and admissible, but nothing to say. A depleted subject with no
	/// action that fits is a gap in the domain, not a quiet success.
	NothingToSay,
}

/// Binds a domain to its calibration, its constraints, and its selector.
///
/// Four type parameters and no trait objects: a deployed binary knows all four,
/// so the whole decision monomorphises into one function.
pub struct Engine<D, C, A, S> {
	admissibility: A,
	selector: S,
	marker: PhantomData<fn() -> (D, C)>,
}

impl<D, C, A, S> Engine<D, C, A, S>
where
	D: Domain,
	C: Calibration<D>,
	A: Admissibility<D>,
	S: Selector<D>,
{
	pub const fn new(admissibility: A, selector: S) -> Self {
		Self {
			admissibility,
			selector,
			marker: PhantomData,
		}
	}

	#[must_use]
	pub const fn admissibility(&self) -> &A {
		&self.admissibility
	}

	/// Fold a signal into the charge, and say when to look again.
	///
	/// The returned instant is the answer to "when could this subject possibly
	/// become eligible", which is what the caller stores and the waker queries.
	pub fn observe(&self, charge: &mut Charge<D>, signal: &D::Signal, at: DateTime<Utc>) -> DateTime<Utc> {
		charge.apply::<C>(signal, at);
		charge.eligible_at::<C>(at)
	}

	/// Decide, given a charge and the last time this subject was interrupted.
	pub fn evaluate(&self, charge: &Charge<D>, at: DateTime<Utc>, last_intervened_at: Option<DateTime<Utc>>) -> Verdict<D::Action, A::Reason> {
		// Refractory first: it is the cheapest check and the only one that is
		// unconditional, so nothing below it can accidentally override it.
		if let Some(last) = last_intervened_at {
			let earliest = last + C::REFRACTORY;
			if at < earliest {
				return Verdict::Wait { until: earliest };
			}
		}

		let eligible_at = charge.eligible_at::<C>(at);
		if at < eligible_at {
			return Verdict::Wait { until: eligible_at };
		}

		let deficits = charge.deficits::<C>(at);
		let Some(action) = self.selector.select(&deficits) else {
			return Verdict::NothingToSay;
		};

		match self.admissibility.admit(at, &action) {
			Ok(()) => Verdict::Intervene(action),
			Err(reason) => Verdict::Suppressed {
				reason,
				// Constraints are transient by nature — quiet hours end, a
				// session finishes — so the retry is soon rather than a
				// recomputed crossing that has already happened.
				retry_at: at + C::REFRACTORY.min(Duration::hours(1)),
			},
		}
	}

	/// Record that an intervention went out: recharge, and report when the
	/// subject could next become eligible.
	pub fn intervened(&self, charge: &mut Charge<D>, at: DateTime<Utc>) -> DateTime<Utc> {
		charge.recharge::<C>(at);
		(at + C::REFRACTORY).max(charge.eligible_at::<C>(at))
	}
}
