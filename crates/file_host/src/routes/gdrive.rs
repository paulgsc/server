use crate::handlers::gdrive_images as routes;
use crate::{AppState, CacheStore, Config, FileHostError};
use axum::routing::get;
use axum::{http::Method, Router};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

pub fn get_gdrive_image(config: Arc<Config>) -> Result<Router, FileHostError> {
	let cors = CorsLayer::new().allow_methods([Method::GET]).allow_origin(Any);
	let state = CacheStore::new(config.clone())?;
	let app_state = AppState {
		cache_store: state,
		config: config.clone(),
	};

	Ok(
		Router::new()
			.route("/gdrive/image/:image_id", get(routes::serve_gdrive_image))
			.layer(cors)
			.with_state(Arc::new(app_state)),
	)
}
