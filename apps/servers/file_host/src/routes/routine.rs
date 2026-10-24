//! `/api/v1/routine` — what the owner's cloud routines left for the phone
//! through the Jobs mailbox, other than the Jobs digests (`routes::jobs`).
//!
//! ```text
//! GET /routine/files  → { files, runs, at }: the reflections routine's
//!                       newest word sets and dramas home holds, newest
//!                       first, at most 20, and its newest runs, at most 31
//! PUT /routine/opened → { change }: which of its lessons the phone opened,
//!                       names only, filed for the mailbox
//! ```
//!
//! **Trust model:** a subject route, as `routes::jobs`: a request without a
//! session is `401`. The `mailbox` poller is the files' one writer; the one
//! write here is a filing for home's outbox (docs/identity.md invariant 19).

use crate::auth::AuthContext;
use crate::handlers::routine as handlers;
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

pub fn routine<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new().get("/routine/files", handlers::get_files).put("/routine/opened", handlers::put_opened);
	Module::versioned("routine", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::PUT, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION])
}
