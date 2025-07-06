use crate::handlers::tab_metadata as routes;
use crate::{FileHostError, WebSocketFsm};
use axum::routing::post;
use axum::{http::Method, Router};
use tower_http::cors::{Any, CorsLayer};

pub fn post_now_playing(ws: WebSocketFsm) -> Result<Router, FileHostError> {
	let cors = CorsLayer::new()
		.allow_origin(Any) // Allow any origin (including extensions)
		.allow_methods([Method::GET, Method::POST])
		.allow_headers(Any);

	Ok(Router::new().route("/now-playing", post(routes::now_playing)).layer(cors).with_state(ws))
}
