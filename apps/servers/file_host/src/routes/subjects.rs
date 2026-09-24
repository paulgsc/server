use crate::handlers::subjects as handlers;
use crate::routes::cors::allowlisted_cors;
use crate::{AppState, Config};
use axum::{
	extract::FromRef,
	http::{
		header::{ACCEPT, AUTHORIZATION},
		Method,
	},
	routing::get,
	Router,
};

/// Per-subject reads (#289, TEL4). `me`, not an id: see `handlers::subjects`.
///
/// Bounded by the catalogue — one row per activity the subject has an outcome
/// for — not by how much they have played; see `outcome_repo::STATS_CEILING`.
pub fn subjects<S>(config: &Config) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let cors = allowlisted_cors(config, vec![Method::GET, Method::OPTIONS], vec![ACCEPT, AUTHORIZATION]);

	Router::new().route("/subjects/me/stats", get(handlers::stats)).layer(cors)
}
