use axum::{body::Body, extract::State, http::Response, middleware::Next, response::IntoResponse};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
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

pub struct TokenBucketRateLimiter {
	max_tokens: u32,
	refill_rate_per_ms: u64, // tokens per millisecond (scaled by 1000 for precision)
	tokens: AtomicU32,
	last_refill: AtomicU64, // timestamp in milliseconds
}

impl TokenBucketRateLimiter {
	#[must_use]
	pub fn new(max_tokens: u32) -> Self {
		Self::new_with_refill_period(max_tokens, 60_000) // 60 seconds default
	}

	#[must_use]
	pub fn new_with_refill_period(max_tokens: u32, refill_period_ms: u64) -> Self {
		// Calculate refill rate: how many tokens per millisecond (scaled by 1000)
		// This ensures we don't lose precision for small rates
		let refill_rate_per_ms = (u64::from(max_tokens) * 1000) / refill_period_ms;

		Self {
			max_tokens,
			refill_rate_per_ms: refill_rate_per_ms.max(1), // Ensure at least 1/1000 token per ms
			tokens: AtomicU32::new(max_tokens),            // start with full bucket
			last_refill: AtomicU64::new(Self::current_time_millis()),
		}
	}

	fn current_time_millis() -> u64 {
		SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis().try_into().unwrap_or(u64::MAX)
	}

	fn refill_tokens(&self, now: u64) {
		const MAX_ATTEMPTS: usize = 3;

		for _ in 0..MAX_ATTEMPTS {
			let last_refill = self.last_refill.load(Ordering::Acquire);

			// Check if enough time has passed to warrant a refill
			let time_elapsed = now.saturating_sub(last_refill);
			if time_elapsed < 10 {
				// Less than 10ms, skip refill
				break;
			}

			// Calculate tokens to add (scaled by 1000 for precision)
			let tokens_to_add_scaled = time_elapsed * self.refill_rate_per_ms;
			let tokens_to_add = u32::try_from(tokens_to_add_scaled / 1000).unwrap_or(self.max_tokens);

			if tokens_to_add == 0 {
				break; // Not enough time elapsed
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
			let new_tokens = (current_tokens + tokens_to_add).min(self.max_tokens);

			// Only update if there's actually a change
			if new_tokens == current_tokens {
				break;
			}

			match self.tokens.compare_exchange_weak(current_tokens, new_tokens, Ordering::AcqRel, Ordering::Acquire) {
				Ok(_) => break, // Successfully added tokens
				Err(_) => {}
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
		let now = Self::current_time_millis();

		// Refill tokens based on elapsed time
		self.refill_tokens(now);

		// Try to consume a token
		const MAX_ATTEMPTS: usize = 10;
		for _ in 0..MAX_ATTEMPTS {
			let current_tokens = self.tokens.load(Ordering::Acquire);
			if current_tokens == 0 {
				return Ok(false); // No tokens available
			}

			// Try to atomically decrement
			match self.tokens.compare_exchange_weak(current_tokens, current_tokens - 1, Ordering::AcqRel, Ordering::Acquire) {
				Ok(_) => return Ok(true), // Successfully consumed a token
				Err(_) => continue,       // CAS failed, retry
			}
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

pub async fn rate_limit_middleware(
	State(limiter): State<Arc<TokenBucketRateLimiter>>,
	request: axum::http::Request<Body>,
	next: Next,
) -> Result<impl IntoResponse, RateLimitError> {
	match limiter.allow_request() {
		Ok(true) => Ok(next.run(request).await),
		Ok(false) => Err(RateLimitError::RateLimited),
		Err(e) => Err(e),
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
		assert!(available >= 4 && available <= 6); // Allow some variance

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
}
