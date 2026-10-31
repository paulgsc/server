//! `/api/v1/study-notes` — the owner's study notes (paulgsc/some-ui#2180).
//!
//! HF5: the text the phone keeps beside a soundbite or a `LeetType` spoken
//! answer, one closed list of kinds (`study_note_repo::Kind`). The audio of
//! each is a voice note (`routes::voice`).
//!
//! ```text
//! PUT /study-notes/:kind/:id  → { change, note }: the body is the note itself
//! ```
//!
//! **Trust model:** a subject route, as `routes::voice`. The handler takes a
//! `SubjectId`, so a request without a session is `401` and a write from an
//! untrusted origin `403`. The phone reaches it with its pairing's bearer
//! token.
//!
//! **Append-only, and kept for good.** A `PUT` adds a note or changes
//! nothing, and other bytes under a held id are a `409`; there is no
//! `DELETE`. See `handlers::study_note` and `study_note_repo`.

use crate::auth::AuthContext;
use crate::handlers::study_note as handlers;
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

pub fn study_notes<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new().put("/study-notes/:kind/:id", handlers::put_note);

	Module::versioned("study_notes", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::PUT, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION])
}
