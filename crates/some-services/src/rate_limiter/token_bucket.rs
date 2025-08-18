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
	refill_rate: u32, // tokens per second
	tokens: AtomicU32,
	last_refill: AtomicU64, // timestamp in milliseconds
}

impl TokenBucketRateLimiter {
	#[must_use]
	pub fn new(max_tokens: u32) -> Self {
		Self {
			max_tokens,
			refill_rate: max_tokens / 60,       // refill over 60 seconds
			tokens: AtomicU32::new(max_tokens), // start with full bucket
			last_refill: AtomicU64::new(Self::current_time_millis()),
		}
	}

	fn current_time_millis() -> u64 {
		SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
	}

	pub fn allow_request(&self) -> Result<bool, RateLimitError> {
		let now = Self::current_time_millis();
		let last_refill = self.last_refill.load(Ordering::Relaxed);

		// Calculate tokens to add based on time elapsed
		let time_elapsed = now.saturating_sub(last_refill);
		let tokens_to_add = (time_elapsed * self.refill_rate as u64) / 1000;

		if tokens_to_add > 0 {
			// Try to update the refill time
			if self.last_refill.compare_exchange_weak(last_refill, now, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
				// We won the race to update timestamp, so add tokens
				let current_tokens = self.tokens.load(Ordering::Relaxed);
				let new_tokens = (current_tokens + tokens_to_add as u32).min(self.max_tokens);
				self.tokens.store(new_tokens, Ordering::Relaxed);
			}
		}

		// Try to consume a token
		loop {
			let current_tokens = self.tokens.load(Ordering::Relaxed);
			if current_tokens == 0 {
				return Ok(false); // No tokens available
			}

			// Try to atomically decrement
			if self
				.tokens
				.compare_exchange_weak(current_tokens, current_tokens - 1, Ordering::Relaxed, Ordering::Relaxed)
				.is_ok()
			{
				return Ok(true); // Successfully consumed a token
			}
			// If CAS failed, retry (another thread modified tokens)
		}
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
