use crate::handlers::audio_files as routes;
use crate::routes::cors::allowlisted_cors;
use crate::{AppState, Config};
use axum::routing::{get, post};
use axum::{
	extract::FromRef,
	http::{
		header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, ETAG, LAST_MODIFIED},
		Method,
	},
	Router,
};

pub fn get_audio<S>(config: &Config) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let cors = allowlisted_cors(config, vec![Method::GET, Method::POST], vec![CONTENT_TYPE, AUTHORIZATION, CACHE_CONTROL, ETAG, LAST_MODIFIED]);

	Router::new()
		// Get specific audio file by ID
		.route("/get_audio/:id", get(routes::get_audio))
		// Search audio files with query parameters
		.route("/search_audio", get(routes::search_audio))
		// Alternative POST endpoint for complex search queries
		.route("/search_audio", post(routes::search_audio_post))
		.layer(cors)
}
