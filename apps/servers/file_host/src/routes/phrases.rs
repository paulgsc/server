//! `/api/v1/phrases` — the Korean phrases a hunch finds (paulgsc/server#1261,
//! KPLAY's KP4; `crate::phrases`).
//!
//! ```text
//! POST /phrases/lookup  → { kind: "hit", phrases: [id, …] }, closest first,
//!                         or { kind: "kept" }: body { words, hunch }, the
//!                         hunch's English words and the phone's name for it;
//!                         a miss is kept in the request that answers it,
//!                         once however often it is sent
//! ```
//!
//! **Trust model:** a paired phone's, as the take library is
//! (`routes::voice`): the handler takes a `Paired`, so a request without a
//! session is `401` and a browser's cookie session `403`.
//!
//! A spoken hunch is heard by the routes that hear every voice note: a note
//! of kind `hunch` whose `ref` is the hunch's name (`PUT /voice-notes/:id`,
//! `POST /voice-notes/:id/fast-track`, `GET /voice-notes/:id/heard`), heard
//! in English and never told a phrase. See `handlers::phrases`.

use crate::auth::AuthContext;
use crate::handlers::phrases as handlers;
use crate::meaning::MeaningContext;
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

pub fn phrases<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
	Option<MeaningContext>: FromRef<S>,
{
	let table = RouteTable::new().post("/phrases/lookup", handlers::post_lookup);

	Module::versioned("phrases", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::POST, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION])
}
