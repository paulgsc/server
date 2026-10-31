//! `/api/v1/voice-notes` — the owner's voice notes, audio included
//! (paulgsc/server#460): what the phone sends home so home can keep each note
//! and transcribe it.
//!
//! ```text
//! PUT /voice-notes/:id?of=&ref=&at=          → { change, note }: the body is
//!                                              the audio itself, its container
//!                                              the request's `Content-Type`
//! GET /voice-notes/:id/transcripts            → { note, chosen, hearings }: a
//!                                              Studio take's hearings that
//!                                              ended, the owner's included,
//!                                              newest first
//! PUT /voice-notes/:id/transcripts/owner/:n   → { change, hearing }: the body
//!                                              is the owner's words for the
//!                                              take, `{ basedOn, words }`
//! ```
//!
//! **A take's words** (paulgsc/some-ui#2322, ST24): the phone reads what home
//! heard in a Studio take, to play it back with its words lit, and keeps the
//! owner's fix as the owner's own transcript, its `n`th save, beside the
//! engines' (`voice_repo::owner`). Only a `studio` note's words are read or
//! written here; any other note's are read by no route.
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
//! **Append-only, and kept for good.** A `PUT` adds a note or a save, or
//! changes nothing, and other bytes, another description or other words
//! under a held id or save are a `409`; there is no `DELETE`. See
//! `handlers::voice` and `voice_repo`.

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
		.get("/voice-notes/:id/transcripts", handlers::get_hearings)
		.put("/voice-notes/:id/transcripts/owner/:n", handlers::put_owner_words);

	Module::versioned("voice", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::PUT, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION])
}
