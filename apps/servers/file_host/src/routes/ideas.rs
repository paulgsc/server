//! `/api/v1/ideas` and `/api/v1/idea-lists` — the owner's ideas, kept on
//! home as one record each, and the lists they pick from.
//!
//! From STUDIO ST26 (paulgsc/server#645; the phone's half paulgsc/some-ui#2453):
//! an idea is one bundle (title, topic, format, keywords, the question set's
//! version and its answers), sent together by Send.
//!
//! ```text
//! GET    /ideas                       → { ideas: [Idea] }: newest begun first, set-aside left out, at most 1,000
//! GET    /ideas/:id                   → Idea, set aside or not
//! PUT    /ideas/:id                   → { change, idea }: the body is the send
//! PUT    /ideas/:id/furnishing        → { change, furnishing }: the body is { furnishing }
//! PUT    /ideas/:id/set-aside         → 204
//! GET    /idea-lists                  → { topics, keywords }
//! PUT    /idea-lists/topics/:name     → { change, topics, keywords }
//! DELETE /idea-lists/topics/:name     → { change, topics, keywords }
//! PUT    /idea-lists/keywords/:name   → { change, topics, keywords }
//! DELETE /idea-lists/keywords/:name   → { change, topics, keywords }
//! ```
//!
//! **Trust model:** a subject route, as `routes::studio_log`. Every handler
//! takes a `SubjectId`, so a request without a session is `401` and a write
//! from an untrusted origin `403`. The phone reaches it with its pairing's
//! bearer token. The lists are the subject's own, so only the owner edits the
//! owner's.
//!
//! **Append-only.** Every write adds a row or changes nothing; a `DELETE` on
//! a list is a mark that takes the entry off, never a delete of a row. What a
//! send may hold is `idea_repo`'s. See `handlers::ideas`.

use crate::auth::AuthContext;
use crate::handlers::ideas as handlers;
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

pub fn ideas<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new()
		.get("/ideas", handlers::get_ideas)
		.get("/ideas/:id", handlers::get_idea)
		.put("/ideas/:id", handlers::put_idea)
		.put("/ideas/:id/furnishing", handlers::put_furnishing)
		.put("/ideas/:id/set-aside", handlers::put_set_aside)
		.get("/idea-lists", handlers::get_lists)
		.put("/idea-lists/topics/:name", handlers::put_topic)
		.delete("/idea-lists/topics/:name", handlers::delete_topic)
		.put("/idea-lists/keywords/:name", handlers::put_keyword)
		.delete("/idea-lists/keywords/:name", handlers::delete_keyword);

	Module::versioned("ideas", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(
		config,
		vec![Method::GET, Method::PUT, Method::DELETE, Method::OPTIONS],
		vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION],
	)
}
