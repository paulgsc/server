use crate::handlers::obs as routes;
use axum::routing::get;
use axum::Router;
use tower_http::cors::CorsLayer;

pub fn get_obs_server() -> Router {
	Router::new()
		// TODO: Add path validation: something about must start with slashes?
		.route("/api/obs/status", get(routes::get_obs_status))
		.route("/ws/obs", get(routes::websocket_handler))
		.layer(CorsLayer::permissive()) // For development - restrict in production
}
