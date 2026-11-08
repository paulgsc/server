use crate::Config;
use axum::http::{header::RETRY_AFTER, HeaderName, HeaderValue, Method};
use tower_http::cors::{AllowOrigin, CorsLayer};

fn origins(config: &Config) -> AllowOrigin {
	AllowOrigin::list(config.allowed_origins.iter().filter_map(|origin| origin.parse::<HeaderValue>().ok()))
}

/// Exposes `Retry-After`, so a browser client refused by the rate limiter
/// can read how long to wait (CORS hides every response header but a few
/// safelisted ones unless named here).
pub fn allowlisted_cors(config: &Config, methods: Vec<Method>, headers: Vec<HeaderName>) -> CorsLayer {
	CorsLayer::new()
		.allow_origin(origins(config))
		.allow_methods(methods)
		.allow_headers(headers)
		.expose_headers([RETRY_AFTER])
}

/// Same allowlist, plus `Access-Control-Allow-Credentials`.
///
/// Separate from [`allowlisted_cors`] rather than a flag on it, because
/// credentialed CORS is a stronger grant than the read routes want and should
/// have to be asked for by name. An explicit origin list is a prerequisite —
/// browsers reject `*` with credentials — which is another reason these routes
/// cannot keep hardcoding a single literal.
pub fn allowlisted_cors_with_credentials(config: &Config, methods: Vec<Method>, headers: Vec<HeaderName>) -> CorsLayer {
	allowlisted_cors(config, methods, headers).allow_credentials(true)
}
