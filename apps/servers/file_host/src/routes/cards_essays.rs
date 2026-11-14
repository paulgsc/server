//! `/api/v1/cards-essays` — a cards essay's video on the phone (CARDS7,
//! #854; C338): the owner watches what home rendered from the essay's cut
//! list.
//!
//! ```text
//! GET /cards-essays/:key/video → the essay's video, streamed, with ranges
//! ```
//!
//! `:key` is an essay's key, `<slug>-<YYYY-MM-DD>`
//! (`cards_essays::pass::is_key`); any other, or one with no video yet, is
//! `404`. The phone finds each key on Studio › Renders' `cards-essay` shelf
//! (`feeds::renders`). Nothing here lets a video go: home keeps it.
//!
//! **Trust model:** a paired phone's only, as `routes::previews`'. Every
//! handler takes a `Paired`, so a request without a session is `401` and a
//! browser's session `403`.

use crate::auth::AuthContext;
use crate::handlers::cards_essays as handlers;
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

pub fn cards_essays<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new().get("/cards-essays/:key/video", handlers::get_video);

	Module::versioned("cards_essays", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION, RANGE])
}
