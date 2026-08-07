use axum::{body::Body, extract::State, http::Response, middleware::Next, response::IntoResponse};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};

#[derive(Clone)]
pub struct SlidingWindowRateLimiter {
	max_requests: usize,
	window_size: Duration,
	request_timestamps: Arc<Mutex<VecDeque<Instant>>>,
}

impl SlidingWindowRateLimiter {
	pub fn new(max_requests: usize) -> Self {
		Self {
			max_requests,
			window_size: Duration::from_secs(60),
			request_timestamps: Arc::new(Mutex::new(VecDeque::new())),
		}
	}

	pub async fn allow_request(&self) -> bool {
		let now = Instant::now();
		let mut timestamps = self.request_timestamps.lock().await;

		// Remove timestamps outside the sliding window
		while let Some(&timestamp) = timestamps.front() {
			if now.duration_since(timestamp) > self.window_size {
				timestamps.pop_front();
			} else {
				break;
			}
		}

		if timestamps.len() < self.max_requests {
			timestamps.push_back(now);
			true
		} else {
			false
		}
	}
}

pub async fn rate_limit_middleware(State(limiter): State<Arc<SlidingWindowRateLimiter>>, request: axum::http::Request<Body>, next: Next) -> impl IntoResponse {
	if limiter.allow_request().await {
		next.run(request).await
	} else {
		Response::builder().status(429).body(Body::from("Rate limit exceeded")).unwrap()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	// These tests previously called `new(Arc::new(Config::parse()))` against a
	// signature that takes `usize`, so the module had not compiled in some
	// time. Rewritten against the real constructor: the limit is now passed
	// directly instead of being read out of ambient config, which also makes
	// the assertions deterministic rather than dependent on whatever
	// `RATE_LIMIT` happened to be set in the environment.

	#[tokio::test]
	async fn allows_requests_up_to_the_limit() {
		let limiter = SlidingWindowRateLimiter::new(3);

		assert!(limiter.allow_request().await);
		assert!(limiter.allow_request().await);
		assert!(limiter.allow_request().await);
	}

	#[tokio::test]
	async fn denies_requests_past_the_limit() {
		let limiter = SlidingWindowRateLimiter::new(2);

		assert!(limiter.allow_request().await);
		assert!(limiter.allow_request().await);
		assert!(!limiter.allow_request().await, "third request should be rejected with a limit of 2");
	}

	/// The old version of this test slept two seconds against a sixty-second
	/// window and asserted the window had expired, so it could only ever have
	/// failed. Uses tokio's paused clock instead: no wall-clock wait, and it
	/// advances past the real window rather than a made-up one.
	#[tokio::test(start_paused = true)]
	async fn allows_requests_again_once_the_window_expires() {
		let limiter = SlidingWindowRateLimiter::new(2);

		assert!(limiter.allow_request().await);
		assert!(limiter.allow_request().await);
		assert!(!limiter.allow_request().await);

		// Strictly greater than `window_size` - the eviction check is `>`.
		tokio::time::advance(Duration::from_secs(61)).await;

		assert!(limiter.allow_request().await, "window should have slid past both earlier requests");
	}
}
