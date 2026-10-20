//! `/api/v1/leetype/sessions` — the owner's `leetype` session log (paulgsc/some-ui
//! `packages/ui/leetype/src/lib/leetype/session-log`): every event of every
//! phone session of incident rounds, per session.
//!
//! ```text
//! GET /leetype/sessions                          → { sessions: [{ session, file, firstAt, lastAt, events }] }: newest first, at most 1,000
//! GET /leetype/sessions/:session/events          → { session, events }: the session, in the order recorded
//! PUT /leetype/sessions/:session/events/:id      → { change, event }: the body is the event itself
//! ```
//!
//! **Trust model:** a subject route, as `routes::jobs`. Every handler takes a
//! `SubjectId`, so a request without a session is `401` and a write from an
//! untrusted origin `403`. The phone reaches it with its pairing's bearer
//! token, from its tapped "Send to home". Not to be confused with
//! `routes::db::leetype` (`/leetype/rounds*`), the corpus-wide round
//! catalogue, which carries no subject.
//!
//! **Append-only.** A `PUT` adds an event or changes nothing, and a different
//! body under a held id is a `409`; there is no `DELETE`. No soundbite comes
//! here: a spoken answer's ticks, prompt, rubric or typed words are a `422`.
//! See `handlers::leetype_log` and `leetype_log_repo`.

use crate::auth::AuthContext;
use crate::handlers::leetype_log as handlers;
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

pub fn leetype_log<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new()
		.get("/leetype/sessions", handlers::get_sessions)
		.get("/leetype/sessions/:session/events", handlers::get_session_events)
		.put("/leetype/sessions/:session/events/:id", handlers::put_event);

	Module::versioned("leetype_log", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::PUT, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION])
}
