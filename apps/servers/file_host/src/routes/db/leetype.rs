//! `/api/v1/leetype` — the `leetype` activity's round corpus, read-only (#327,
//! LTY-SRV3).
//!
//! ```text
//! GET /leetype/rounds       → { version, rounds: [{ id, version, publishedAt, contentHash, witnesses }] }, listed only, bounded
//! GET /leetype/rounds/:id   → one round, verbatim, or a JSON 404
//! ```
//!
//! The same shape of surface as `routes::db::curriculum`, and for the same
//! reason: a round that does not exist is a real `404`, where a static file
//! server behind `try_files … /index.html` answers `200` with a page. Both
//! routes answer `If-None-Match` with `304`: a round's `ETag` is its content
//! hash, and the manifest's is the hash of its listing, which is also its
//! `version`. `witnesses` is each round's `μ` in option order, so a client can
//! sample rounds by proposition without fetching bodies.
//!
//! Read-only: rounds arrive through `import-leetype-rounds` (#326) or the
//! operator's routes (`routes::db::leetype_operator`). The manifest lists only
//! rounds the operator has not retired; a retired round is still served by id.
//! No `SubjectId`: a round is corpus-wide. `ETag` is exposed and
//! `If-None-Match` allowed cross-origin for the reason `routes::db::activities`
//! gives.

use crate::handlers::db::leetype as handlers;
use crate::routes::cors::allowlisted_cors;
use crate::routes::table::{Module, RouteTable};
use crate::Config;
use axum::{
	extract::FromRef,
	http::{
		header::{ACCEPT, CONTENT_TYPE, ETAG, IF_NONE_MATCH},
		Method,
	},
};
use sqlx::SqlitePool;
use tower_http::cors::CorsLayer;

pub fn leetype<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
{
	let table = RouteTable::new()
		.get("/leetype/rounds", handlers::get_rounds)
		.get("/leetype/rounds/:id", handlers::get_round);

	Module::versioned("leetype", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors(config, vec![Method::GET, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT, IF_NONE_MATCH]).expose_headers([ETAG])
}
