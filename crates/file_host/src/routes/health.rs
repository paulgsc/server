use crate::handlers::health as routes;
use crate::FileHostError;
use axum::routing::get;
use axum::{http::Method, Router};
use tower_http::cors::{Any, CorsLayer};

pub fn get_health() -> Result<Router, FileHostError> {
	let cors = CorsLayer::new()
		.allow_origin(Any) // Allow any origin (including extensions)
		.allow_methods([Method::GET])
		.allow_headers(Any);

	Ok(Router::new().route("/health", get(routes::health)).layer(cors))
}
