//! The algorithm itself, as pure functions of a client's TAT and the time.
//!
//! See `rate_limiter.rs`'s module docs for what the TAT is and why.

use std::{num::NonZeroU32, time::Duration};

/// How much a client may send: `burst` requests at once, refilling at one
/// request per `interval`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quota {
	burst: u32,
	interval: u64,
}

impl Quota {
	/// `limit` requests per `period`, all of which may arrive at once.
	///
	/// The interval is `period / limit`, rounded *up* to the nanosecond, so
	/// when the division is inexact the limiter errs strict by under a
	/// nanosecond per request rather than generous.
	#[must_use]
	pub fn per_period(limit: NonZeroU32, period: Duration) -> Self {
		let interval = period.as_nanos().max(1).div_ceil(u128::from(limit.get()));
		Self {
			burst: limit.get(),
			interval: u64::try_from(interval).unwrap_or(u64::MAX),
		}
	}

	#[must_use]
	pub const fn burst(&self) -> u32 {
		self.burst
	}

	#[must_use]
	pub const fn interval(&self) -> Duration {
		Duration::from_nanos(self.interval)
	}

	/// How far past now a client's TAT may sit and still be admitted.
	///
	/// In `u128`: `burst × interval` overflows `u64` only for a period over
	/// 584 years, but if it ever did, a saturated tolerance would admit
	/// everything. Wide, it cannot.
	const fn tolerance(&self) -> u128 {
		self.interval as u128 * self.burst as u128
	}
}

/// What the limiter said about one request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
	/// Admitted, with this many more requests admissible right now.
	Allowed { remaining: u32 },
	/// Refused; the same request is admitted once `retry_after` has passed.
	Limited { retry_after: Duration },
}

impl Decision {
	#[must_use]
	pub const fn is_allowed(&self) -> bool {
		matches!(self, Self::Allowed { .. })
	}
}

/// Decides one request at `now` for a client whose TAT is `tat` (`None`: a
/// client with a full bucket). Returns the decision and, if admitted, the
/// client's new TAT. A refusal leaves the TAT as it was, so a client hammering
/// while refused does not push its own return further out.
///
/// Fails closed: a TAT that would pass `u64::MAX` is a refusal, not a wrap.
pub(super) fn decide(tat: Option<u64>, now: u64, quota: &Quota) -> (Decision, Option<u64>) {
	let next = u128::from(tat.map_or(now, |tat| tat.max(now))) + u128::from(quota.interval);
	let ahead = next - u128::from(now);
	let tolerance = quota.tolerance();
	let Ok(next) = u64::try_from(next) else {
		return (Decision::Limited { retry_after: Duration::MAX }, None);
	};
	if ahead > tolerance {
		let wait = u64::try_from(ahead - tolerance).unwrap_or(u64::MAX);
		return (
			Decision::Limited {
				retry_after: Duration::from_nanos(wait),
			},
			None,
		);
	}
	let remaining = u32::try_from((tolerance - ahead) / u128::from(quota.interval)).unwrap_or(u32::MAX);
	(Decision::Allowed { remaining }, Some(next))
}

/// Requests admissible right now for a client whose TAT is `tat`. Reads only.
pub(super) fn available(tat: u64, now: u64, quota: &Quota) -> u32 {
	let owed = u128::from(tat.saturating_sub(now));
	u32::try_from(quota.tolerance().saturating_sub(owed) / u128::from(quota.interval)).unwrap_or(u32::MAX)
}

/// Whether a client's bucket is full, so forgetting it changes nothing.
pub(super) const fn is_full(tat: u64, now: u64) -> bool {
	tat <= now
}
