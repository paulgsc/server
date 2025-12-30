use crate::handlers::tab_metadata as routes;
use crate::AppState;
use axum::routing::post;
use axum::{extract::FromRef, http::Method, Router};
use tower_http::cors::{Any, CorsLayer};

pub fn post_now_playing<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let cors = CorsLayer::new()
		.allow_origin(Any) // Allow any origin (including extensions)
		.allow_methods([Method::GET, Method::POST])
		.allow_headers(Any);

	Router::new().route("/now-playing", post(routes::now_playing)).layer(cors)
}
