use crate::handlers::outcomes as handlers;
use crate::routes::cors::allowlisted_cors;
use crate::routes::table::{Module, RouteTable};
use crate::{AppState, Config};
use axum::{
	extract::FromRef,
	http::{
		header::{AUTHORIZATION, CONTENT_TYPE},
		Method,
	},
};
use tower_http::cors::CorsLayer;

/// Where an applet reports how one block of a session went (#287, TEL2).
///
/// One route: the variety is in the body — see `handlers::outcomes` for what
/// it writes, what it derives, and what it refuses.
pub fn outcomes<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	Module::versioned("outcomes", RouteTable::new().post("/outcomes", handlers::record)).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors(config, vec![Method::POST, Method::OPTIONS], vec![CONTENT_TYPE, AUTHORIZATION])
}
