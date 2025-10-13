/// Trait to determine if an error should trigger circuit breaker logic
pub trait IsRetryable {
	fn is_retryable(&self) -> bool;
}

pub struct ResilienceBuilder<S> {
	service: S,
}

impl<S> ResilienceBuilder<S> {
	pub fn new(service: S) -> Self {
		Self { service }
	}

	pub fn with_timeout(self, timeout: Duration) -> ResilienceBuilder<tower::timeout::Timeout<S>> {
		ResilienceBuilder {
			service: tower::timeout::Timeout::new(self.service, timeout),
		}
	}

	pub fn with_rate_limit(self, requests_per_second: u32) -> ResilienceBuilder<RateLimitService<S>> {
		ResilienceBuilder {
			service: RateLimitLayer::new(requests_per_second).layer(self.service),
		}
	}

	pub fn with_circuit_breaker(self, failure_threshold: u32, timeout: Duration, reset_timeout: Duration) -> ResilienceBuilder<CircuitBreakerService<S>>
	where
		S::Error: IsRetryable,
	{
		ResilienceBuilder {
			service: CircuitBreakerLayer::new(failure_threshold, timeout, reset_timeout).layer(self.service),
		}
	}

	pub fn with_validation(self) -> ResilienceBuilder<ValidationService<S>> {
		ResilienceBuilder {
			service: ValidationLayer.layer(self.service),
		}
	}

	pub fn build(self) -> S {
		self.service
	}

	pub fn boxed_clone(self) -> BoxCloneService<S::Request, S::Response, S::Error>
	where
		S: Service + Clone + Send + 'static,
		S::Request: 'static,
		S::Response: 'static,
		S::Error: 'static,
		S::Future: Send + 'static,
	{
		tower::util::BoxCloneService::new(self.service)
	}
}
