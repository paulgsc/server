//! `/api/v1/jobs` — the owner's Jobs record (paulgsc/some-ui
//! `apps/www/src/lib/jobs`): every event of what was done with each day's job
//! picks, and what came of it.
//!
//! ```text
//! GET /jobs              → { events }: the record, in the order recorded
//! PUT /jobs/events/:id   → { change, event }: the body is the event itself
//! GET /jobs/digests      → { digests }: the daily job digests home holds,
//!                          oldest first, at most 7; `?after=YYYY-MM-DD` for
//!                          the days after it
//! ```
//!
//! **Trust model:** a subject route, as `routes::aph`. Every handler takes a
//! `SubjectId`, so a request without a session is `401` and a write from an
//! untrusted origin `403`. The phone reaches it with its pairing's bearer
//! token, once its person has opted in to keeping Jobs on home.
//!
//! **Append-only.** A `PUT` adds an event or changes nothing, and a different
//! body under a held id is a `409`; there is no `DELETE`. See
//! `handlers::jobs` and `jobs_repo`.
//!
//! **Digests** are read only: the `mailbox` poller is their one writer.

use crate::auth::AuthContext;
use crate::handlers::jobs as handlers;
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

pub fn jobs<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new()
		.get("/jobs", handlers::get_record)
		.put("/jobs/events/:id", handlers::put_event)
		.get("/jobs/digests", handlers::get_digests);

	Module::versioned("jobs", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::PUT, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION])
}
