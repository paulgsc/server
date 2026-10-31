//! `/api/v1/korean` — the owner's Korean record (paulgsc/some-ui#2100): the
//! word sets the phone studies on and what it records as they are read,
//! one kind per store the phone keeps (`korean_repo::Kind`).
//!
//! ```text
//! GET    /korean/:kind       → { entries }: each standing entry of the kind
//! PUT    /korean/:kind/:key  → { change, entry }: the body is the entry itself
//! DELETE /korean/:kind/:key  → { change, entry }: a revision that removes it
//! ```
//!
//! **Trust model:** a subject route, as `routes::aph`. Every handler takes
//! a `SubjectId`, so a request without a session is `401` and a write from an
//! untrusted origin `403`. The phone reaches it with its pairing's bearer
//! token.
//!
//! **Append-only.** A `PUT` or `DELETE` adds a revision or changes nothing;
//! nothing is ever erased but with the account. See `handlers::korean` and
//! `korean_repo`.

use crate::auth::AuthContext;
use crate::handlers::korean as handlers;
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

pub fn korean<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new()
		.get("/korean/:kind", handlers::get_entries)
		.put("/korean/:kind/:key", handlers::put_entry)
		.delete("/korean/:kind/:key", handlers::delete_entry);

	Module::versioned("korean", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(
		config,
		vec![Method::GET, Method::PUT, Method::DELETE, Method::OPTIONS],
		vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION],
	)
}
