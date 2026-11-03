use crate::handlers::subjects as handlers;
use crate::routes::cors::allowlisted_cors_with_credentials;
use crate::routes::table::{Module, RouteTable};
use crate::{AppState, Config};
use axum::{
	extract::FromRef,
	http::{
		header::{ACCEPT, AUTHORIZATION},
		Method,
	},
};
use sqlx::SqlitePool;
use tower_http::cors::CorsLayer;

/// Per-subject reads (#289, TEL4). `me`, not an id: see `handlers::subjects`.
///
/// ```text
/// GET /subjects/me/stats → { activities, history }: the recommender's numbers
/// GET /subjects/me/kept  → { kinds }: how many of each kind of record home
///                          keeps for good, and since when, to a paired
///                          phone (paulgsc/some-ui#2519)
/// ```
///
/// `stats` is bounded by the catalogue — one row per activity the subject has
/// an outcome for — not by how much they have played; see
/// `outcome_repo::STATS_CEILING`. `kept` answers at most one row per kind
/// (`storage_repo::kept::Record`). It takes a `Paired` phone, as
/// `routes::voice`'s reads do, so a browser's cookie session is a `403`.
pub fn subjects<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
	SqlitePool: FromRef<S>,
	crate::auth::AuthContext: FromRef<S>,
{
	let table = RouteTable::new().get("/subjects/me/stats", handlers::stats).get("/subjects/me/kept", handlers::kept);
	Module::versioned("subjects", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::OPTIONS], vec![ACCEPT, AUTHORIZATION])
}
