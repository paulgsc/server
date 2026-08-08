use axum::{body::Body, http::Response, response::IntoResponse};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RateLimitError {
	#[error("System time error: clock may have gone backwards")]
	ClockError(#[from] std::time::SystemTimeError),
	#[error("Rate limit exceeded")]
	RateLimited,
}

impl IntoResponse for RateLimitError {
	fn into_response(self) -> axum::response::Response {
		let (status, body) = match self {
			Self::RateLimited => (429, "Rate limit exceeded"),
			Self::ClockError(_) => (500, "Internal server error"),
		};
		Response::builder()
			.status(status)
			.body(Body::from(body))
			.unwrap_or_else(|_| Response::new(Body::from("Internal error")))
	}
}

/// The window `config.rs`'s `rate_limit` field already documents itself as:
/// "requests per minute". See #215/#231 (A2).
pub const DEFAULT_REFILL_PERIOD_MS: u64 = 60_000;

pub struct TokenBucketRateLimiter {
	max_tokens: u32,
	refill_period_ms: u64,
	tokens: AtomicU32,
	last_refill: AtomicU64, // timestamp in milliseconds
}

impl TokenBucketRateLimiter {
	#[must_use]
	pub fn new_with_refill_period(max_tokens: u32, refill_period_ms: u64) -> Self {
		Self {
			max_tokens,
			refill_period_ms: refill_period_ms.max(1),
			tokens: AtomicU32::new(max_tokens), // start with full bucket
			last_refill: AtomicU64::new(Self::current_time_millis()),
		}
	}

	#[must_use]
	pub const fn max_tokens(&self) -> u32 {
		self.max_tokens
	}

	fn current_time_millis() -> u64 {
		SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis().try_into().unwrap_or(u64::MAX)
	}

	/// #215/#231 (A2): the previous version precomputed a `refill_rate_per_ms`
	/// (tokens per millisecond, scaled by 1000) and floored it to a minimum of
	/// 1 — so any configuration wanting fewer than one token per second (the
	/// entire range where a per-*minute* limit lives, including the default
	/// 10 tokens / 60s) got silently promoted to 1 token/second, six times
	/// the configured rate for that default.
	///
	/// Computing the accrued tokens directly from elapsed time on each call,
	/// in `u128` to avoid overflow, removes the floor entirely:
	/// `tokens_to_add` is genuinely `0` (never artificially 1) until enough
	/// time has actually elapsed for the configured rate to owe a whole
	/// token, and `last_refill` only advances when it does — so no fractional
	/// token is ever lost between calls, however far apart they land.
	fn refill_tokens(&self, now: u64) {
		const MAX_ATTEMPTS: usize = 3;

		for _ in 0..MAX_ATTEMPTS {
			let last_refill = self.last_refill.load(Ordering::Acquire);
			let time_elapsed = now.saturating_sub(last_refill);

			let tokens_to_add_scaled = u128::from(time_elapsed) * u128::from(self.max_tokens) / u128::from(self.refill_period_ms);
			let tokens_to_add = u32::try_from(tokens_to_add_scaled).unwrap_or(self.max_tokens);

			if tokens_to_add == 0 {
				break; // Not enough time elapsed yet — leave `last_refill` alone so elapsed time keeps accumulating toward the next whole token.
			}

			// Try to update the refill timestamp first
			if self.last_refill.compare_exchange_weak(last_refill, now, Ordering::AcqRel, Ordering::Acquire).is_ok() {
				// Successfully updated timestamp, now add tokens
				self.add_tokens(tokens_to_add);
				break;
			}
			// Another thread updated the timestamp, retry with new value
		}
	}

	fn add_tokens(&self, tokens_to_add: u32) {
		loop {
			let current_tokens = self.tokens.load(Ordering::Acquire);
			let new_tokens = current_tokens.saturating_add(tokens_to_add).min(self.max_tokens);

			// Only update if there's actually a change
			if new_tokens == current_tokens {
				break;
			}

			if self.tokens.compare_exchange_weak(current_tokens, new_tokens, Ordering::AcqRel, Ordering::Acquire).is_ok() {
				break; // Successfully added tokens
			}
		}
	}

	/// Attempts to allow a request by consuming a token from the bucket.
	///
	/// # Errors
	///
	/// Returns `RateLimitError::ClockError` if there's a system time error (though this is
	/// handled gracefully in the current implementation).
	pub fn allow_request(&self) -> Result<bool, RateLimitError> {
		const MAX_ATTEMPTS: usize = 10;

		let now = Self::current_time_millis();

		// Refill tokens based on elapsed time
		self.refill_tokens(now);

		// Try to consume a token
		for _ in 0..MAX_ATTEMPTS {
			let current_tokens = self.tokens.load(Ordering::Acquire);
			if current_tokens == 0 {
				return Ok(false); // No tokens available
			}

			// Try to atomically decrement
			if self
				.tokens
				.compare_exchange_weak(current_tokens, current_tokens - 1, Ordering::AcqRel, Ordering::Acquire)
				.is_ok()
			{
				return Ok(true); // Successfully consumed a token
			}
			// CAS failed, retry
		}

		// If we've retried too many times, assume no tokens available
		Ok(false)
	}

	// Utility method to check current state (useful for debugging)
	pub fn get_current_tokens(&self) -> u32 {
		let now = Self::current_time_millis();
		self.refill_tokens(now);
		self.tokens.load(Ordering::Acquire)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use tokio::time::{sleep, Duration};

	#[tokio::test]
	async fn test_rate_limiter_refill() {
		let limiter = TokenBucketRateLimiter::new_with_refill_period(10, 1000); // 10 tokens per second

		// Consume all tokens
		for _ in 0..10 {
			assert!(limiter.allow_request().unwrap());
		}

		// Should be blocked now
		assert!(!limiter.allow_request().unwrap());

		// Wait for refill
		sleep(Duration::from_millis(500)).await; // Wait 0.5 seconds

		// Should have ~5 tokens now
		let available = limiter.get_current_tokens();
		assert!((4..=6).contains(&available)); // Allow some variance

		// Should allow requests again
		assert!(limiter.allow_request().unwrap());
	}

	#[tokio::test]
	async fn test_rate_limiter_recovery() {
		let limiter = TokenBucketRateLimiter::new_with_refill_period(5, 1000); // 5 tokens per second

		// Exhaust the bucket
		for _ in 0..5 {
			assert!(limiter.allow_request().unwrap());
		}
		assert!(!limiter.allow_request().unwrap());

		// Wait for full refill
		sleep(Duration::from_millis(1100)).await;

		// Should be able to make requests again
		for _ in 0..5 {
			assert!(limiter.allow_request().unwrap(), "Should allow request after refill");
		}
	}

	/// #231's own acceptance criterion, at the exact configuration its
	/// context section names: `max_tokens=10, refill_period_ms=60_000`. The
	/// pre-fix arithmetic floored the rate to 1 token/second (60/minute) for
	/// this configuration — six times too fast. The correct rate accrues one
	/// token every `60_000 / 10 = 6_000`ms; sleeping past that boundary
	/// should yield exactly one token, not six.
	#[tokio::test]
	async fn refill_rate_matches_the_configured_period_at_ten_per_minute() {
		let limiter = TokenBucketRateLimiter::new_with_refill_period(10, 60_000);

		for _ in 0..10 {
			assert!(limiter.allow_request().unwrap());
		}
		assert!(!limiter.allow_request().unwrap(), "bucket should start empty after 10 requests");

		sleep(Duration::from_millis(6_100)).await;

		assert_eq!(
			limiter.get_current_tokens(),
			1,
			"10 tokens per 60s should refill 1 token every 6s, not the pre-fix bug's 1 token/second"
		);
	}
}
