//! `/api/v1/studio` — the owner's Studio time log and friction reports.
//!
//! From paulgsc/some-ui ST9 (#2257): each sitting's screens entered and left,
//! the page hidden and shown, the sitting opened and closed, and how each
//! sitting went.
//!
//! ```text
//! GET /studio/sittings                       → { sittings: [{ sitting, kind, firstAt, lastAt, events }] }: newest first, at most 1,000
//! GET /studio/sittings/:sitting/events       → { sitting, events }: the sitting, in the order recorded
//! PUT /studio/sittings/:sitting/events/:id   → { change, event }: the body is the event itself
//! GET /studio/friction                       → { reports }: newest first, at most 1,000
//! PUT /studio/friction/:id                   → { change, report }: the body is the report itself
//! ```
//!
//! **Trust model:** a subject route, as `routes::leetype_log`. Every handler
//! takes a `SubjectId`, so a request without a session is `401` and a write
//! from an untrusted origin `403`. The phone reaches it with its pairing's
//! bearer token.
//!
//! **Append-only.** A `PUT` adds an entry or changes nothing, and a different
//! body under a held id is a `409`; there is no `DELETE`. What an entry may
//! hold is `studio_log_repo`'s. See `handlers::studio_log`.

use crate::auth::AuthContext;
use crate::handlers::studio_log as handlers;
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

pub fn studio_log<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new()
		.get("/studio/sittings", handlers::get_sittings)
		.get("/studio/sittings/:sitting/events", handlers::get_sitting_events)
		.put("/studio/sittings/:sitting/events/:id", handlers::put_event)
		.get("/studio/friction", handlers::get_reports)
		.put("/studio/friction/:id", handlers::put_report);

	Module::versioned("studio_log", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::PUT, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION])
}
