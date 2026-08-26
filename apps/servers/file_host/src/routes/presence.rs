use crate::{handlers::presence as handlers, routes::cors::allowlisted_cors, AppState, Config};
use axum::{
	extract::FromRef,
	http::{
		header::{AUTHORIZATION, CONTENT_TYPE},
		Method,
	},
	routing::post,
	Router,
};

/// Paths are declared relative: `main.rs` nests this under `API_V1_BASE_PATH`.
///
/// CORS mirrors `routes::push::push` rather than hardcoding an origin: this
/// is written from the same browser-facing study app, on the same secure
/// context `push`'s subscribe route already requires.
pub fn presence<S>(config: &Config) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let cors = allowlisted_cors(config, vec![Method::POST, Method::OPTIONS], vec![CONTENT_TYPE, AUTHORIZATION]);

	Router::new()
		// POST /presence/lease → "I am looking at this, right now."
		.route("/presence/lease", post(handlers::observe_lease))
		.layer(cors)
}
