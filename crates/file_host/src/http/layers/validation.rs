use validator::{Validate, ValidationErrors};

#[derive(Clone)]
pub struct ValidationLayer;

impl<S> Layer<S> for ValidationLayer {
	type Service = ValidationService<S>;

	fn layer(&self, service: S) -> Self::Service {
		ValidationService { inner: service }
	}
}

#[derive(Clone)]
pub struct ValidationService<S> {
	inner: S,
}

impl<S, Request> Service<Request> for ValidationService<S>
where
	S: Service<Request> + Clone + Send + 'static,
	S::Future: Send,
	Request: Validate + Send + 'static,
{
	type Response = S::Response;
	type Error = ValidationError<S::Error>;
	type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

	fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
		self.inner.poll_ready(cx).map_err(ValidationError::Inner)
	}

	fn call(&mut self, req: Request) -> Self::Future {
		// Validate the request
		if let Err(validation_errors) = req.validate() {
			return Box::pin(async move { Err(ValidationError::Invalid(validation_errors)) });
		}

		let mut service = self.inner.clone();
		Box::pin(async move { service.call(req).await.map_err(ValidationError::Inner) })
	}
}

#[derive(Debug)]
pub enum ValidationError<E> {
	Invalid(ValidationErrors),
	Inner(E),
}

impl<E: std::fmt::Display> std::fmt::Display for ValidationError<E> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Invalid(e) => write!(f, "Validation failed: {}", e),
			Self::Inner(e) => write!(f, "Inner service error: {}", e),
		}
	}
}

impl<E: std::error::Error + 'static> std::error::Error for ValidationError<E> {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			Self::Invalid(e) => Some(e),
			Self::Inner(e) => Some(e),
		}
	}
}
