//! `/api/v1/voice-notes` — the owner's voice notes, audio included
//! (paulgsc/server#460): what the phone sends home so home can keep each note
//! and transcribe it.
//!
//! ```text
//! PUT /voice-notes/:id?of=&ref=&at=  → { change, note }: the body is the
//!                                      audio itself, its container the
//!                                      request's `Content-Type`
//! ```
//!
//! **Trust model:** a subject route, as `routes::jobs`. The handler takes a
//! `SubjectId`, so a request without a session is `401` and a write from an
//! untrusted origin `403`. The phone reaches it with its pairing's bearer
//! token.
//!
//! **Append-only, and kept for good.** A `PUT` adds a note or changes
//! nothing, and other bytes or another description under a held id are a
//! `409`; there is no `DELETE`. See `handlers::voice` and `voice_repo`.

use crate::auth::AuthContext;
use crate::handlers::voice as handlers;
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

pub fn voice<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new().put("/voice-notes/:id", handlers::put_note);

	Module::versioned("voice", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::PUT, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION])
}
