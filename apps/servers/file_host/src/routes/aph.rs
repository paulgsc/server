//! `/api/v1/aph` — the owner's aph record (paulgsc/some-ui `packages/ui/aph`):
//! entries of a figure of mine against a figure the other side reports, and
//! the settings they are read against.
//!
//! ```text
//! GET /aph               → { settings, entries }: the record as it stands
//! PUT /aph/entries/:id   → { change, entry }: the body is the entry itself
//! PUT /aph/settings      → { change, settings }: the body is the settings
//! ```
//!
//! **Trust model:** a subject route, as `routes::shelf`. Every handler takes
//! a `SubjectId`, so a request without a session is `401` and a write from an
//! untrusted origin `403`. The phone reaches it with its pairing's bearer
//! token, once its person has opted in to keeping aph on home.
//!
//! **Append-only.** A `PUT` adds a revision or changes nothing; there is no
//! `DELETE`. See `handlers::aph` and `aph_repo`.

use crate::auth::AuthContext;
use crate::handlers::aph as handlers;
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

pub fn aph<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new()
		.get("/aph", handlers::get_record)
		.put("/aph/entries/:id", handlers::put_entry)
		.put("/aph/settings", handlers::put_settings);

	Module::versioned("aph", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::PUT, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION])
}
