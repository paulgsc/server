use crate::handlers::health as routes;
use crate::AppState;
use axum::routing::get;
use axum::{extract::FromRef, http::Method, Router};
use tower_http::cors::{Any, CorsLayer};

pub fn get_health<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let cors = CorsLayer::new()
		.allow_origin(Any) // Allow any origin (including extensions)
		.allow_methods([Method::GET])
		.allow_headers(Any);

	Router::new().route("/health", get(routes::health)).layer(cors)
}
