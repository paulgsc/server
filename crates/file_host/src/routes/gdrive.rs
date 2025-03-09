use crate::handlers::read_attributions as routes;
use crate::{AppState, CacheStore, Config, FileHostError};
use axum::routing::get;
use axum::{http::Method, Router};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

pub fn get_gdrive_file(config: Arc<Config>) -> Result<Router, FileHostError> {
	let cors = CorsLayer::new().allow_methods([Method::GET]).allow_origin(Any);
	let state = CacheStore::new(config.clone())?;
	let app_state = AppState {
		cache_store: state,
		config: config.clone(),
	};

	Ok(Router::new().route("/gdrive/:file_id", get(routes::get)).layer(cors).with_state(Arc::new(app_state)))
}
