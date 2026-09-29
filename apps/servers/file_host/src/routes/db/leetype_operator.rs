//! `/api/v1/leetype/operator` — the operator's hand on the `leetype` rounds
//! the server serves.
//!
//! ```text
//! GET  /leetype/operator/rounds              → { rounds: OperatorRound[] }, retired included, bounded
//! PUT  /leetype/operator/rounds/:id          → { change, round }: write one round, `{ body }`
//! POST /leetype/operator/rounds/:id/retire   → the round, out of the manifest
//! POST /leetype/operator/rounds/:id/restore  → the round, back in it
//! ```
//!
//! `import-leetype-rounds` (#326) stays the way to bring the exported corpus
//! across in bulk; these routes change one round without a shell. Both write
//! through `RoundRepository::upsert`, so a round's content hash, version,
//! `published_at` and witness rows mean the same thing whichever way it
//! arrived. Retiring unlists a round without deleting it (`GET
//! /leetype/rounds/:id` still serves it); restoring lists it again. Neither
//! moves a version, and no round write announces anything: rounds do not feed
//! the study nudge's publication log.
//!
//! The paths sit under `/leetype/operator/`, beside `/leetype/rounds/`, so
//! they shadow no round id.
//!
//! **Trust model:** an operator only, exactly as for the lesson operator
//! routes: every handler takes `auth::operator::Operator`, a passkey session
//! whose subject is listed in `OPERATOR_SUBJECTS` (`401` without a session,
//! `403` otherwise; `docs/study-nudge.md`, "Trust model, stated plainly").
//!
//! The server stays blind to what a round is: `body` is stored verbatim, and
//! read only as far as `leetype_round_repo::parse_round` reads it.

use crate::auth::AuthContext;
use crate::handlers::db::leetype_operator as handlers;
use crate::routes::cors::allowlisted_cors_with_credentials;
use crate::routes::table::{Module, RouteTable};
use crate::Config;
use axum::{
	extract::FromRef,
	http::{
		header::{ACCEPT, CONTENT_TYPE},
		Method,
	},
};
use sqlx::SqlitePool;
use tower_http::cors::CorsLayer;

pub fn leetype_operator<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new()
		.get("/leetype/operator/rounds", handlers::get_rounds)
		.put("/leetype/operator/rounds/:id", handlers::put_round)
		.post("/leetype/operator/rounds/:id/retire", handlers::retire_round)
		.post("/leetype/operator/rounds/:id/restore", handlers::restore_round);

	Module::versioned("leetype_operator", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::PUT, Method::POST, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT])
}
