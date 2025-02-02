use crate::handlers::read_attributions as routes;
use crate::CacheStore;
use crate::Config;
use axum::routing::get;
use axum::{http::Method, Router};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

pub fn get_attributions(config: Arc<Config>) -> Router {
	let cors = CorsLayer::new().allow_methods([Method::GET]).allow_origin(Any);
	let state = CacheStore::new(config).unwrap();

	Router::new().route("/gsheet/:sheet_id", get(routes::get)).layer(cors).with_state(Arc::new(state))
}
