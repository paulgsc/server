use axum::{body::Body, extract::State, http::Response, middleware::Next, response::IntoResponse};
use some_services::rate_limiter::{RateLimitError, TokenBucketRateLimiter};
use std::sync::Arc;

pub async fn rate_limit_middleware(State(limiter): State<Arc<TokenBucketRateLimiter>>, request: axum::http::Request<Body>, next: Next) -> impl IntoResponse {
	let allow_request = match limiter.allow_request() {
		Ok(val) => val,
		Err(RateLimitError::RateLimited) => {
			return Response::builder().status(429).body(Body::from("Rate limit exceeded")).unwrap();
		}
		Err(err) => {
			return Response::builder().status(500).body(Body::from(format!("Internal error: {}", err))).unwrap();
		}
	};

	if allow_request {
		next.run(request).await
	} else {
		Response::builder().status(429).body(Body::from("Rate limit exceeded")).unwrap()
	}
}
