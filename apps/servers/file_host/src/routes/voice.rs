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
//! GET /voice-notes/:id/heard          → { heard: null | { text, language,
//!                                      model, heardAt } }: what home heard
//!                                      in the note, or null while it has
//!                                      not heard it yet
//! PUT /voice-notes/:id/set-aside?at=  → 204, no body: the owner let the
//!                                      note go; home sets it aside and
//!                                      keeps it (idempotent, the first
//!                                      mark stands)
//! GET /voice-notes/:id/audio          → { id, contentType, sha256, audio }:
//!                                      a Rust Rounds note's audio as
//!                                      base64, to a paired phone, with its
//!                                      SHA-256 as the ETag (`304` on
//!                                      `If-None-Match`), or a JSON 404
//! ```
//!
//! The Drive read (paulgsc/some-ui#2401, HF11) answers from what the
//! build-journal session told home through the mailbox
//! (`voice_repo::on_drive`); home never reaches Drive. The heard read
//! (paulgsc/some-ui#2428, MIC10) answers from the transcribe worker's rows
//! (`voice_repo::transcript::heard`), so a screen that wants a take's words
//! gets them from home rather than from a recognizer on the phone. The
//! set-aside mark (paulgsc/some-ui#2372, MIC2; the owner's D3 on epic #2368:
//! Let go sets a take aside and keeps it) is `voice_repo::set_aside`'s, its
//! own table beside the note, so no note is rewritten.
//!
//! The audio read is the one route that gives a note's bytes back, for a
//! `rust-round` note only (`voice_repo::audio` says why).
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
//! token. The audio read takes a `Paired` phone, as `routes::rust_rounds`
//! does, so a browser's cookie session is a `403` there too.
//!
//! **Append-only, and kept for good.** A `PUT` adds a note or changes
//! nothing, and other bytes or another description under a held id are a
//! `409`; there is no `DELETE`, and setting a note aside deletes nothing.
//! See `handlers::voice` and `voice_repo`.

use crate::auth::AuthContext;
use crate::handlers::voice as handlers;
use crate::routes::cors::allowlisted_cors_with_credentials;
use crate::routes::table::{Module, RouteTable};
use crate::Config;
use axum::{
	extract::FromRef,
	http::{
		header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, ETAG, IF_NONE_MATCH},
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
		.get("/voice-notes/on-drive", handlers::get_on_drive)
		.get("/voice-notes/:id/heard", handlers::get_heard)
		.put("/voice-notes/:id/set-aside", handlers::put_set_aside)
		.get("/voice-notes/:id/audio", handlers::get_audio);

	Module::versioned("voice", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(
		config,
		vec![Method::GET, Method::PUT, Method::OPTIONS],
		vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION, IF_NONE_MATCH],
	)
	.expose_headers([ETAG])
}
