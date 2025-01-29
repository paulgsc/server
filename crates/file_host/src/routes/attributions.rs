use crate::handlers::read_attributions as routes;
use crate::Config;
use axum::routing::get;
use axum::{http::Method, Router};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

pub fn get_attributions(_config: Arc<Config>) -> Router {
	let cors = CorsLayer::new().allow_methods([Method::GET]).allow_origin(Any);

	Router::new().route(&format!("/"), get(routes::get)).layer(cors)
}
