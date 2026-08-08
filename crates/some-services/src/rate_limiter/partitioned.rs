//! `PartitionedTokenBucketLimiter` — #215/#231 (A2).
//!
//! [`TokenBucketRateLimiter`] alone is a single bucket for the entire
//! process: one client bursting its allowance denies every other client
//! until refill catches up, which makes it a global concurrency cap with
//! extra steps rather than a rate limiter. This wraps one bucket per
//! `client_id` in a [`DashMap`], keyed the same way `ConnectionGuard`
//! (`ws-conn-manager`) already keys WebSocket admission — the peer IP — so
//! HTTP and WS agree on what "a client" is.
//!
//! `client_id` is a partition key into this map, not a metric label: the
//! cardinality concern that keeps per-client identity off
//! `rate_limit_decisions_total` (unbounded label cardinality on a counter
//! that never expires) does not apply to a bucket store that evicts idle
//! entries — see [`Self::evict_idle`].

use super::token_bucket::{RateLimitError, TokenBucketRateLimiter};
use dashmap::DashMap;

pub struct PartitionedTokenBucketLimiter {
	max_tokens: u32,
	refill_period_ms: u64,
	buckets: DashMap<String, TokenBucketRateLimiter>,
}

impl PartitionedTokenBucketLimiter {
	#[must_use]
	pub fn new(max_tokens: u32, refill_period_ms: u64) -> Self {
		Self {
			max_tokens,
			refill_period_ms,
			buckets: DashMap::new(),
		}
	}

	/// # Errors
	///
	/// Returns `RateLimitError::ClockError` if the system clock error
	/// propagated from the underlying bucket's own `allow_request`.
	pub fn allow_request(&self, client_id: &str) -> Result<bool, RateLimitError> {
		let bucket = self
			.buckets
			.entry(client_id.to_string())
			.or_insert_with(|| TokenBucketRateLimiter::new_with_refill_period(self.max_tokens, self.refill_period_ms));
		bucket.allow_request()
	}

	/// Drops buckets currently sitting at full capacity.
	///
	/// Safe by construction: a full bucket's owner gets an identical fresh
	/// bucket on their next request anyway (`or_insert_with` above starts
	/// every new entry full), so evicting one can never make a client more
	/// restricted than leaving it in place. The only observable effect is
	/// bounding this map's size under churning client identities — #231's
	/// own acceptance criterion — rather than one entry per IP ever seen,
	/// forever.
	#[must_use]
	pub fn evict_idle(&self) -> usize {
		let before = self.buckets.len();
		self.buckets.retain(|_, bucket| bucket.get_current_tokens() < bucket.max_tokens());
		before - self.buckets.len()
	}

	#[must_use]
	pub fn active_clients(&self) -> usize {
		self.buckets.len()
	}

	/// Average tokens currently available across tracked clients — #230's
	/// "useful for debugging" gauge, aggregated rather than per-client (see
	/// this module's own header on why `client_id` stays a partition key,
	/// not a metric label). An idle limiter (no tracked clients) reports
	/// full capacity, matching what a first request would actually see.
	#[must_use]
	pub fn average_tokens_available(&self) -> f64 {
		if self.buckets.is_empty() {
			return f64::from(self.max_tokens);
		}

		let total: u64 = self.buckets.iter().map(|entry| u64::from(entry.get_current_tokens())).sum();
		#[allow(clippy::cast_precision_loss)]
		{
			total as f64 / self.buckets.len() as f64
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use tokio::time::{sleep, Duration};

	#[test]
	fn one_client_exhausting_its_bucket_does_not_affect_another() {
		let limiter = PartitionedTokenBucketLimiter::new(2, 60_000);

		assert!(limiter.allow_request("client-a").unwrap());
		assert!(limiter.allow_request("client-a").unwrap());
		assert!(!limiter.allow_request("client-a").unwrap(), "client-a should be exhausted");

		assert!(limiter.allow_request("client-b").unwrap(), "client-b has its own bucket and should be unaffected");
		assert!(limiter.allow_request("client-b").unwrap());
	}

	#[test]
	fn average_tokens_available_reports_full_capacity_when_idle() {
		let limiter = PartitionedTokenBucketLimiter::new(10, 60_000);
		assert!((limiter.average_tokens_available() - 10.0).abs() < f64::EPSILON);
		assert_eq!(limiter.active_clients(), 0);
	}

	#[tokio::test]
	async fn evict_idle_reclaims_buckets_back_at_full_capacity() {
		let limiter = PartitionedTokenBucketLimiter::new(1, 50); // 1 token / 50ms, fast enough to refill within a test

		for i in 0..10 {
			let client_id = "client-".to_string() + &i.to_string();
			assert!(limiter.allow_request(&client_id).unwrap());
		}
		assert_eq!(limiter.active_clients(), 10);
		assert_eq!(limiter.evict_idle(), 0, "freshly emptied buckets are not idle and must not be reclaimed");

		sleep(Duration::from_millis(60)).await;

		assert_eq!(limiter.evict_idle(), 10, "buckets refilled to capacity are safe to evict");
		assert_eq!(limiter.active_clients(), 0, "the map must not grow without bound under churning client identities");
	}
}
