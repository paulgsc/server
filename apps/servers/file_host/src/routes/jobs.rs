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
//! GET /jobs/runs         → { runs }: the jobs routine's run records home
//!                          holds, newest first, at most 7
//! PUT /jobs/notice       → { change }: tell this phone, through FCM, when a
//!                          morning run's record arrives; `{ token }`
//! DELETE /jobs/notice    → { change }: stop telling it; `{ token }`
//! GET /jobs/files        → { files }: the decisions and scouts files filed
//!                          here, newest first, at most 30, each with the
//!                          mailbox's answer and when, or null while pending
//! PUT /jobs/files/:name  → { change }: a decisions or scouts file, for the
//!                          mailbox poller to hand on
//! ```
//!
//! **Trust model:** a subject route, as `routes::aph`. Every handler takes a
//! `SubjectId`, so a request without a session is `401` and a write from an
//! untrusted origin `403`. The phone reaches it with its pairing's bearer
//! token once paired.
//!
//! **Append-only.** A `PUT` adds an event or changes nothing, and a different
//! body under a held id is a `409`; there is no `DELETE`. See
//! `handlers::jobs` and `jobs_repo`.
//!
//! **Digests** and **runs** are read only: the `mailbox` poller is their one
//! writer.
//! **Files** are filed once per name and never replaced. They are text: a
//! voice note is named by its id, never carried. Reading them back gives
//! each one's name, times and the mailbox's answer, never its body.

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
		.get("/jobs/digests", handlers::get_digests)
		.get("/jobs/runs", handlers::get_runs)
		.put("/jobs/notice", handlers::put_notice)
		.delete("/jobs/notice", handlers::delete_notice)
		.get("/jobs/files", handlers::get_files)
		.put("/jobs/files/:name", handlers::put_file);

	Module::versioned("jobs", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(
		config,
		vec![Method::GET, Method::PUT, Method::DELETE, Method::OPTIONS],
		vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION],
	)
}
