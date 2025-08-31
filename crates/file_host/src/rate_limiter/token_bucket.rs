use axum::{body::Body, extract::State, http::Response, middleware::Next, response::IntoResponse};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct TokenBucketRateLimiter {
	max_tokens: u32,
	refill_rate: u32, // tokens per second
	tokens: AtomicU32,
	last_refill: AtomicU64, // timestamp in milliseconds
}

impl TokenBucketRateLimiter {
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

	pub fn allow_request(&self) -> bool {
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
				return false; // No tokens available
			}

			// Try to atomically decrement
			if self
				.tokens
				.compare_exchange_weak(current_tokens, current_tokens - 1, Ordering::Relaxed, Ordering::Relaxed)
				.is_ok()
			{
				return true; // Successfully consumed a token
			}
			// If CAS failed, retry (another thread modified tokens)
		}
	}
}

pub async fn rate_limit_middleware(State(limiter): State<Arc<TokenBucketRateLimiter>>, request: axum::http::Request<Body>, next: Next) -> impl IntoResponse {
	if limiter.allow_request() {
		next.run(request).await
	} else {
		Response::builder().status(429).body(Body::from("Rate limit exceeded")).unwrap()
	}
}
