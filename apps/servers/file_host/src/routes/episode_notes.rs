//! `/api/v1/feeds/:feed/notes` — the owner's notes on the feeds' episodes
//! (SFEED6: paulgsc/some-ui#2745), left on the phone's Feedback page.
//!
//! ```text
//! PUT /feeds/:feed/notes/:id → { change, note }: a note at a moment of one
//!                              of the feed's episodes, kept on home and
//!                              filed for the mailbox's `feedback` folder;
//!                              `note.writtenAt` comes back to the millisecond
//! GET /feeds/:feed/notes     → { notes: [{ id, episode, atMs, categories,
//!                              take, writtenAt, storedAt }] }: oldest first
//! ```
//!
//! `:feed` is `call` or `playoff-delta` (`feeds::Feed`); any other is `404`.
//!
//! **Trust model:** a subject route, as `routes::routine`: a paired phone's
//! or a browser's session. A request without a session is `401`, and a write
//! from an untrusted origin `403`. Notes are append-only (docs/identity.md
//! invariant 38): there is no delete and no switch (the owner's answer on
//! paulgsc/some-ui#2749).

use crate::auth::AuthContext;
use crate::handlers::episode_notes as handlers;
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

pub fn episode_notes<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new()
		.get("/feeds/:feed/notes", handlers::get_notes)
		.put("/feeds/:feed/notes/:id", handlers::put_note);

	Module::versioned("episode_notes", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::PUT, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION])
}
