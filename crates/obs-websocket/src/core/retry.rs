use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct RetryConfig {
	pub max_consecutive_failures: usize,
	pub initial_delay: Duration,
	pub max_delay: Duration,
	pub backoff_multiplier: f64,
	pub circuit_breaker_timeout: Duration,
}

impl Default for RetryConfig {
	fn default() -> Self {
		Self {
			max_consecutive_failures: 10,
			initial_delay: Duration::from_secs(1),
			max_delay: Duration::from_secs(60),
			backoff_multiplier: 1.5,
			circuit_breaker_timeout: Duration::from_secs(15),
		}
	}
}

pub struct RetryPolicy {
	config: RetryConfig,
	consecutive_failures: usize,
	current_delay: Duration,
	circuit_breaker_opened_at: Option<Instant>,
}

impl RetryPolicy {
	pub fn new(config: RetryConfig) -> Self {
		let current_delay = config.initial_delay;
		Self {
			config,
			consecutive_failures: 0,
			current_delay,
			circuit_breaker_opened_at: None,
		}
	}

	pub async fn should_retry(&mut self) -> bool {
		// Check circuit breaker
		if let Some(opened_at) = self.circuit_breaker_opened_at {
			if opened_at.elapsed() < self.config.circuit_breaker_timeout {
				tokio::time::sleep(Duration::from_secs(30)).await;
				return true;
			} else {
				// Reset circuit breaker
				self.circuit_breaker_opened_at = None;
				self.consecutive_failures = 0;
				self.current_delay = self.config.initial_delay;
			}
		}

		self.consecutive_failures += 1;

		if self.consecutive_failures >= self.config.max_consecutive_failures {
			// Open circuit breaker
			self.circuit_breaker_opened_at = Some(Instant::now());
			tracing::error!("Circuit breaker opened due to {} consecutive failures", self.consecutive_failures);
			return true;
		}

		// Exponential backoff
		tokio::time::sleep(self.current_delay).await;
		self.current_delay = Duration::from_millis((self.current_delay.as_millis() as f64 * self.config.backoff_multiplier) as u64).min(self.config.max_delay);

		true
	}

	pub fn reset(&mut self) {
		self.consecutive_failures = 0;
		self.current_delay = self.config.initial_delay;
		self.circuit_breaker_opened_at = None;
	}
}
