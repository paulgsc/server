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
pub struct RateLimitLayer {
	limiter: Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
}

impl RateLimitLayer {
	pub fn new(requests_per_second: u32) -> Self {
		let quota = Quota::per_second(std::num::NonZeroU32::new(requests_per_second).unwrap());
		let limiter = Arc::new(RateLimiter::direct(quota));
		Self { limiter }
	}

	pub fn per_minute(requests_per_minute: u32) -> Self {
		let quota = Quota::per_minute(std::num::NonZeroU32::new(requests_per_minute).unwrap());
		let limiter = Arc::new(RateLimiter::direct(quota));
		Self { limiter }
	}
}

impl<S> Layer<S> for RateLimitLayer {
	type Service = RateLimitService<S>;

	fn layer(&self, service: S) -> Self::Service {
		RateLimitService {
			inner: service,
			limiter: self.limiter.clone(),
		}
	}
}

#[derive(Clone)]
pub struct RateLimitService<S> {
	inner: S,
	limiter: Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
}

impl<S, Request> Service<Request> for RateLimitService<S>
where
	S: Service<Request> + Clone + Send + 'static,
	S::Future: Send,
	Request: Send + 'static,
{
	type Response = S::Response;
	type Error = RateLimitError<S::Error>;
	type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

	fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
		self.inner.poll_ready(cx).map_err(RateLimitError::Inner)
	}

	fn call(&mut self, req: Request) -> Self::Future {
		if self.limiter.check().is_err() {
			return Box::pin(async move { Err(RateLimitError::Limited) });
		}

		let mut service = self.inner.clone();
		Box::pin(async move { service.call(req).await.map_err(RateLimitError::Inner) })
	}
}

#[derive(Debug)]
pub enum RateLimitError<E> {
	Limited,
	Inner(E),
}

impl<E: std::fmt::Display> std::fmt::Display for RateLimitError<E> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Limited => write!(f, "Rate limit exceeded"),
			Self::Inner(e) => write!(f, "Inner service error: {}", e),
		}
	}
}

impl<E: std::error::Error + 'static> std::error::Error for RateLimitError<E> {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			Self::Inner(e) => Some(e),
			Self::Limited => None,
		}
	}
}
