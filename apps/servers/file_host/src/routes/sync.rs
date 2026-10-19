//! `/api/v1/sync` — many writes of one kept record in one request, over
//! each record's own storage.
//!
//! ```text
//! POST /sync/:collection/batch → { results }: each item as its own PUT answers it
//! ```
//!
//! `:collection` is `jobs` (the Jobs record's events) or `aph` (aph's entry
//! revisions). A batch is one request against the rate limit, where the
//! routes it stands for are one per item.
//!
//! **Trust model:** a subject route, as `routes::jobs` and `routes::aph`:
//! the handler takes a `SubjectId`, and a paired phone's batch is refused
//! once its person stopped sending that record (`handlers::sharing`). See
//! `handlers::sync`.

use crate::auth::AuthContext;
use crate::handlers::sync as handlers;
use crate::routes::cors::allowlisted_cors_with_credentials;
use crate::routes::table::{Module, RouteTable};
use crate::Config;
use axum::{
	extract::FromRef,
	http::{
		header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE},
		Method,
	},
};
use sqlx::SqlitePool;
use tower_http::cors::CorsLayer;

pub fn sync<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new().post("/sync/:collection/batch", handlers::write_batch);

	Module::versioned("sync", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::POST, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION])
}
