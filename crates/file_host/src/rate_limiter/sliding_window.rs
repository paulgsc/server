use crate::Config;
use axum::{body::Body, extract::State, http::Response, middleware::Next, response::IntoResponse};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct SlidingWindowRateLimiter {
	max_requests: usize,
	window_size: Duration,
	request_timestamps: Arc<Mutex<VecDeque<Instant>>>,
}

impl SlidingWindowRateLimiter {
	pub fn new(config: Arc<Config>) -> Self {
		Self {
			max_requests: config.rate_limit as usize,
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
	use clap::Parser;
	use tokio::time::sleep;

	#[tokio::test]
	async fn test_allow_request_within_limit() {
		dotenv::dotenv().ok();
		let limiter = SlidingWindowRateLimiter::new(Arc::new(Config::parse()));

		assert!(limiter.allow_request().await);
		assert!(limiter.allow_request().await);
		assert!(limiter.allow_request().await);
	}

	#[tokio::test]
	async fn test_deny_request_exceeding_limit() {
		let limiter = SlidingWindowRateLimiter::new(Arc::new(Config::parse()));

		assert!(limiter.allow_request().await);
		assert!(limiter.allow_request().await);
		assert!(!limiter.allow_request().await);
	}

	#[tokio::test]
	async fn test_request_allowed_after_window_expires() {
		let limiter = SlidingWindowRateLimiter::new(Arc::new(Config::parse()));

		assert!(limiter.allow_request().await);
		assert!(limiter.allow_request().await);
		assert!(!limiter.allow_request().await);

		sleep(Duration::from_secs(2)).await;
		assert!(limiter.allow_request().await);
	}
}
