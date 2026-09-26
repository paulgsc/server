use crate::handlers::subjects as handlers;
use crate::routes::cors::allowlisted_cors;
use crate::routes::table::{Module, RouteTable};
use crate::{AppState, Config};
use axum::{
	extract::FromRef,
	http::{
		header::{ACCEPT, AUTHORIZATION},
		Method,
	},
};
use tower_http::cors::CorsLayer;

/// Per-subject reads (#289, TEL4). `me`, not an id: see `handlers::subjects`.
///
/// Bounded by the catalogue — one row per activity the subject has an outcome
/// for — not by how much they have played; see `outcome_repo::STATS_CEILING`.
pub fn subjects<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	Module::versioned("subjects", RouteTable::new().get("/subjects/me/stats", handlers::stats)).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors(config, vec![Method::GET, Method::OPTIONS], vec![ACCEPT, AUTHORIZATION])
}
