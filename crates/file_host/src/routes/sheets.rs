use crate::handlers::read_sheets as routes;
use crate::{AppState, CacheStore, Config, FileHostError};
use axum::routing::get;
use axum::{
	http::{
		header::{AUTHORIZATION, CONTENT_TYPE},
		{HeaderValue, Method},
	},
	Router,
};
use std::sync::Arc;
use tower_http::cors::CorsLayer;

pub fn get_sheets(config: Arc<Config>) -> Result<Router, FileHostError> {
	let cors = CorsLayer::new()
		.allow_origin("http://nixos.local:6006".parse::<HeaderValue>().unwrap())
		.allow_methods([Method::GET])
		.allow_headers([CONTENT_TYPE, AUTHORIZATION])
		.allow_credentials(true);
	let state = CacheStore::new(config.clone())?;
	let app_state = AppState {
		cache_store: state,
		config: config.clone(),
	};

	Ok(
		Router::new()
			// TODO: Add path validation: something about must start with slashes?
			.route("/get_attributions/:sheet_id", get(routes::get_attributions))
			.route("/get_video_chapters/:sheet_id", get(routes::get_video_chapters))
			.route("/get_gantt/:sheet_id", get(routes::get_gantt))
			.route("/get_nfl_tennis/:sheet_id", get(routes::get_nfl_tennis))
			.layer(cors)
			.with_state(Arc::new(app_state)),
	)
}
