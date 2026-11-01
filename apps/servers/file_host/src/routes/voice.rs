//! `/api/v1/voice-notes` — the owner's voice notes, audio included
//! (paulgsc/server#460): what the phone sends home so home can keep each note
//! and transcribe it.
//!
//! ```text
//! PUT /voice-notes/:id?of=&ref=&at=  → { change, note }: the body is the
//!                                      audio itself, its container the
//!                                      request's `Content-Type`
//! GET /voice-notes/on-drive?ids=      → { onDrive: [{ note, listedAt }] }:
//!                                      which of the phone's Studio takes
//!                                      home knows Drive holds too
//! ```
//!
//! The read (paulgsc/some-ui#2401, HF11) answers from what the build-journal
//! session told home through the mailbox (`voice_repo::on_drive`); home
//! never reaches Drive.
//!
//! A game bite is a note said for `game` (`ref` the game's id), whose
//! query also carries the bite's fields (`voice_repo::SaidBite` lists them),
//! and whose answer's `note` carries it as `bite`.
//! A Studio take is a note said for `studio`, its `ref` the take
//! (`<project>:<shot>:<take>`), and carries nothing more.
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
	let table = RouteTable::new()
		.put("/voice-notes/:id", handlers::put_note)
		.get("/voice-notes/on-drive", handlers::get_on_drive);

	Module::versioned("voice", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::PUT, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION])
}
