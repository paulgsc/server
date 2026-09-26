use crate::handlers::utterance as routes;
use crate::routes::table::{Module, RouteTable};
use crate::{AppState, Config};
use axum::{extract::FromRef, http::Method};
use tower_http::cors::{Any, CorsLayer};

pub fn post_utterance<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	Module::versioned("utterance", RouteTable::new().post("/utter", routes::utterance)).with_cors(cors)
}

fn cors(_: &Config) -> CorsLayer {
	CorsLayer::new()
		.allow_origin(Any) // Allow any origin (including extensions)
		.allow_methods([Method::GET, Method::POST])
		.allow_headers(Any)
}
