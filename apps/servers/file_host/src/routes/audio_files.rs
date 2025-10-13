use crate::handlers::audio_files as routes;
use crate::AppState;
use axum::routing::{get, post};
use axum::{
	extract::FromRef,
	http::{
		header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, ETAG, LAST_MODIFIED},
		{HeaderValue, Method},
	},
	Router,
};
use tower_http::cors::CorsLayer;

pub fn get_audio<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let cors = CorsLayer::new()
		.allow_origin("http://nixos.local:6006".parse::<HeaderValue>().unwrap())
		.allow_methods([Method::GET, Method::POST])
		.allow_headers([CONTENT_TYPE, AUTHORIZATION, CACHE_CONTROL, ETAG, LAST_MODIFIED])
		.allow_credentials(true);

	Router::new()
		// Get specific audio file by ID
		.route("/get_audio/:id", get(routes::get_audio))
		// Search audio files with query parameters
		.route("/search_audio", get(routes::search_audio))
		// Alternative POST endpoint for complex search queries
		.route("/search_audio", post(routes::search_audio_post))
		.layer(cors)
}
