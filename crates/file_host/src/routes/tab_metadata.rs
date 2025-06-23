use crate::handlers::tab_metadata as routes;
use crate::{AppState, CacheStore, Config, FileHostError};
use axum::routing::post;
use axum::{http::Method, Router};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

pub fn post_now_playing(config: Arc<Config>) -> Result<Router, FileHostError> {
	let cors = CorsLayer::new()
		.allow_origin(Any) // Allow any origin (including extensions)
		.allow_methods([Method::GET, Method::POST])
		.allow_headers(Any);
	let state = CacheStore::new(config.clone())?;
	let app_state = AppState {
		cache_store: state,
		config: config.clone(),
	};

	Ok(Router::new().route("/now-playing", post(routes::now_playing)).layer(cors).with_state(Arc::new(app_state)))
}
