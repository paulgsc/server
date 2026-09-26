use crate::handlers::signals as handlers;
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

/// The ingress for domain events.
///
/// One route, because the interesting variety is in the body's type rather than
/// in the URL space: adding a signal is a variant in `study_domain`, not an
/// endpoint here.
pub fn signals<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	Module::versioned("signals", RouteTable::new().post("/signals", handlers::observe)).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors(config, vec![Method::POST, Method::OPTIONS], vec![CONTENT_TYPE, AUTHORIZATION])
}
