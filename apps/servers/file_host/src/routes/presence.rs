use crate::{
	handlers::presence as handlers,
	routes::cors::allowlisted_cors,
	routes::table::{Module, RouteTable},
	AppState, Config,
};
use axum::{
	extract::FromRef,
	http::{
		header::{AUTHORIZATION, CONTENT_TYPE},
		Method,
	},
};
use tower_http::cors::CorsLayer;

/// Paths are declared relative: `main.rs` nests this under `API_V1_BASE_PATH`.
///
/// CORS mirrors `routes::push::push` rather than hardcoding an origin: this
/// is written from the same browser-facing study app, on the same secure
/// context `push`'s subscribe route already requires.
pub fn presence<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let table = RouteTable::new()
		// POST /presence/lease → "I am looking at this, right now."
		.post("/presence/lease", handlers::observe_lease);

	Module::versioned("presence", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors(config, vec![Method::POST, Method::OPTIONS], vec![CONTENT_TYPE, AUTHORIZATION])
}
