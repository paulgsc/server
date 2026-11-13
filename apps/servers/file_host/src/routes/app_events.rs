//! `/api/v1/app-events` — what the app saw happen as it ran.
//!
//! From paulgsc/some-ui CK2 (#2381): each obligation the phone ran to an
//! ending (a take today) says how it ended, how long it took, which screen
//! it came from and which build. One route for every family of event, so
//! ENGAGE's engagement events (paulgsc/some-ui ENG2, #2535) ride it too.
//!
//! ```text
//! PUT /app-events/:id        → { change, event }: the body is the event itself
//! GET /app-events/summary    → { since, families, outcomes, last, partial }: ?days=1..90, default 7
//! GET /app-events/weeks/:week → { week, weeks, seen }: an ISO week (2026-W46) and its three comparison weeks
//! ```
//!
//! **Trust model:** a subject route, as `routes::studio_log`. Every handler
//! takes a `SubjectId`, so a request without a session is `401` and a write
//! from an untrusted origin `403`. The phone reaches it with its pairing's
//! bearer token.
//!
//! **Append-only.** A `PUT` adds an event or changes nothing, and a different
//! body under a held id is a `409`; there is no `DELETE`. What an event may
//! hold is `app_event_repo`'s. See `handlers::app_events`.

use crate::auth::AuthContext;
use crate::handlers::app_events as handlers;
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

pub fn app_events<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new()
		.get("/app-events/summary", handlers::get_summary)
		.get("/app-events/weeks/:week", handlers::get_weekly)
		.put("/app-events/:id", handlers::put_event);

	Module::versioned("app_events", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::PUT, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION])
}
