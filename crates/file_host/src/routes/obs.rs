use crate::handlers::obs as routes;
use axum::routing::get;
use axum::Router;
use tower_http::cors::{Any, CorsLayer};

pub fn get_obs_server() -> Router {
	let cors = CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any);

	Router::new()
		// TODO: Add path validation: something about must start with slashes?
		.route("/api/obs/status", get(routes::get_obs_status))
		.route("/ws/obs", get(routes::websocket_handler))
		.layer(cors) // For development - restrict in production
}
