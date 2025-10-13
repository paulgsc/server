use std::{
	fmt::Debug,
	future::Future,
	pin::Pin,
	sync::Arc,
	task::{Context, Poll},
	time::{Duration, SystemTime},
};

use governor::{
	clock::DefaultClock,
	state::{InMemoryState, NotKeyed},
	Quota, RateLimiter,
};
use tower::{layer::Layer, util::BoxCloneService, Service, ServiceBuilder};
use tracing::{info, warn};

#[derive(Clone)]
pub struct CircuitBreakerLayer {
	failure_threshold: u32,
	timeout: Duration,
	reset_timeout: Duration,
}

impl CircuitBreakerLayer {
	pub fn new(failure_threshold: u32, timeout: Duration, reset_timeout: Duration) -> Self {
		Self {
			failure_threshold,
			timeout,
			reset_timeout,
		}
	}
}

impl<S> Layer<S> for CircuitBreakerLayer {
	type Service = CircuitBreakerService<S>;
	fn layer(&self, inner: S) -> Self::Service {
		CircuitBreakerService::new(inner, self.failure_threshold, self.timeout, self.reset_timeout)
	}
}

#[derive(Clone)]
pub struct CircuitBreakerService<S> {
	inner: S,
	state: Arc<tokio::sync::Mutex<CircuitBreakerState>>,
	failure_threshold: u32,
	reset_timeout: Duration,
}

#[derive(Debug)]
struct CircuitBreakerState {
	failures: u32,
	last_failure: Option<SystemTime>,
	is_open: bool,
}

impl<S> CircuitBreakerService<S> {
	fn new(service: S, failure_threshold: u32, _timeout: Duration, reset_timeout: Duration) -> Self {
		Self {
			inner: service,
			state: Arc::new(tokio::sync::Mutex::new(CircuitBreakerState {
				failures: 0,
				last_failure: None,
				is_open: false,
			})),
			failure_threshold,
			reset_timeout,
		}
	}
}

impl<S, Req> TowerService<Req> for CircuitBreakerService<S>
where
	S: TowerService<Req> + Clone + Send + 'static,
	S::Future: Send,
	S::Error: IsRetryable + Send,
	Req: Send + 'static,
{
	type Response = S::Response;
	type Error = CircuitBreakerError<S::Error>;
	type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

	fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
		self.inner.poll_ready(cx).map_err(CircuitBreakerError::Inner)
	}

	fn call(&mut self, req: Req) -> Self::Future {
		let mut svc = self.inner.clone();
		let state = self.state.clone();
		let reset_timeout = self.reset_timeout;
		let failure_threshold = self.failure_threshold;

		Box::pin(async move {
			let mut guard = state.lock().await;
			if guard.is_open {
				if let Some(t) = guard.last_failure {
					if t.elapsed().unwrap_or_default() > reset_timeout {
						guard.is_open = false;
						guard.failures = 0;
						info!("Circuit breaker reset");
					} else {
						return Err(CircuitBreakerError::Open);
					}
				}
			}
			drop(guard);

			match svc.call(req).await {
				Ok(res) => {
					let mut guard = state.lock().await;
					guard.failures = 0;
					Ok(res)
				}
				Err(e) => {
					if e.is_retryable() {
						let mut guard = state.lock().await;
						guard.failures += 1;
						guard.last_failure = Some(SystemTime::now());
						if guard.failures >= failure_threshold {
							guard.is_open = true;
							warn!("Circuit breaker opened after {} failures", guard.failures);
						}
					}
					Err(CircuitBreakerError::Inner(e))
				}
			}
		})
	}
}

#[derive(Debug)]
pub enum CircuitBreakerError<E> {
	Open,
	Inner(E),
}
