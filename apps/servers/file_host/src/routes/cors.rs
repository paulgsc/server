use crate::Config;
use axum::http::{HeaderName, HeaderValue, Method};
use tower_http::cors::{AllowOrigin, CorsLayer};

pub fn allowlisted_cors(config: &Config, methods: Vec<Method>, headers: Vec<HeaderName>) -> CorsLayer {
	let origins: Vec<HeaderValue> = config.allowed_origins.iter().filter_map(|origin| origin.parse().ok()).collect();

	CorsLayer::new().allow_origin(AllowOrigin::list(origins)).allow_methods(methods).allow_headers(headers)
}
