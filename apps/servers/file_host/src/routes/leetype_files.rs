//! `/api/v1/leetype/session-files` — the `LeetType` session files the owner's
//! cloud routine left for the phone through the Jobs mailbox's `leetype`
//! folder (paulgsc/some-ui#2026), as home keeps them.
//!
//! ```text
//! GET /leetype/session-files        → { files: [{ name, title, oneLine, sha256, keptAt }] }: newest first, at most 200
//! GET /leetype/session-files/:name  → the session file itself, verbatim; `ETag` is `sha256`, `304` on `If-None-Match`
//! ```
//!
//! **Trust model:** a subject route, as `routes::routine`: a request without
//! a session is `401`. Read only: the `mailbox` poller is the files' one
//! writer (docs/identity.md invariant 19), and the phone never uploads one.
//! Apart from `routes::leetype_log` (the session log the phone sends home)
//! and `routes::routine` (the reflections routine's files), so neither's
//! reader decides what the other shows.

use crate::auth::AuthContext;
use crate::handlers::leetype_files as handlers;
use crate::routes::cors::allowlisted_cors_with_credentials;
use crate::routes::table::{Module, RouteTable};
use crate::Config;
use axum::{
	extract::FromRef,
	http::{
		header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, ETAG, IF_NONE_MATCH},
		Method,
	},
};
use sqlx::SqlitePool;
use tower_http::cors::CorsLayer;

pub fn leetype_files<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new()
		.get("/leetype/session-files", handlers::get_files)
		.get("/leetype/session-files/:name", handlers::get_file);
	Module::versioned("leetype_files", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION, IF_NONE_MATCH]).expose_headers([ETAG])
}
