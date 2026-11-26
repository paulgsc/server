//! `/api/v1/conversations` — a conversation essay's record and the previews
//! it offers (HOST16-S, paulgsc/server#1243; paulgsc/some-ui#3558).
//!
//! ```text
//! GET  /conversations                                       → { conversations: [{ id, topic, phase, offers, standing }] }, newest first, at most 50
//! GET  /conversations/:id                                   → the record: { schema, id, events }
//! GET  /conversations/:id/previews                          → { previews: [{ preview, about, recommended, variants, approved }], unreadable }
//! GET  /conversations/:id/previews/:preview/:variant/video  → the variant's video, streamed, with ranges
//! POST /conversations/:id/events                            → 201 { kept, events }; 200 for the log's last again; 409 { refused, at }
//! ```
//!
//! **Trust model:** a subject route, as `routes::studio_log`: a paired
//! phone's or a browser's session. Every handler takes a `SubjectId`, so a
//! request without a session is `401` and a write from an untrusted origin
//! `403`.
//!
//! **Append-only.** A `POST` keeps one event at the log's end, once the
//! record's step takes it after the kept log, or nothing; there is no
//! `DELETE`. What the step holds is `conversation_repo::step`'s; what a
//! preview is, `conversations`'.

use crate::auth::AuthContext;
use crate::handlers::conversations as handlers;
use crate::routes::cors::allowlisted_cors_with_credentials;
use crate::routes::table::{Module, RouteTable};
use crate::Config;
use axum::{
	extract::FromRef,
	http::{
		header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, RANGE},
		Method,
	},
};
use sqlx::SqlitePool;
use tower_http::cors::CorsLayer;

pub fn conversations<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new()
		.get("/conversations", handlers::list_conversations)
		.get("/conversations/:id", handlers::get_record)
		.get("/conversations/:id/previews", handlers::get_previews)
		.get("/conversations/:id/previews/:preview/:variant/video", handlers::get_video)
		.post("/conversations/:id/events", handlers::post_event);

	Module::versioned("conversations", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::POST, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION, RANGE])
}
