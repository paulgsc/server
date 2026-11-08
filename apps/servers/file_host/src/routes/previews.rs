//! `/api/v1/previews` — a template's preview on the phone (SFEED12:
//! paulgsc/some-ui#2757): a short video drawn from the template's sample,
//! made on home under the lane, kept a day.
//!
//! ```text
//! POST /previews/:template          → 202 { id, state } for a new one; 200 with the one still to be made
//! GET  /previews/:template          → { preview: null | { id, version?, state, made_at?, expires_at?, bytes?, failed? } }
//! GET  /previews/:template/:id/video → the preview's video, streamed, with ranges
//! ```
//!
//! `:template` is `conference-call`, `playoff-delta-legend` or
//! `playoff-delta-week` (`previews::TEMPLATES`); any other is `404`.
//!
//! **Trust model:** a paired phone's only, as `routes::feed_videos`'. Every
//! handler takes a `Paired`, so a request without a session is `401` and a
//! browser's session `403`.
//!
//! What a preview is, and the prune, are `previews`'s; the answers are
//! `handlers::previews`'s.

use crate::auth::AuthContext;
use crate::handlers::previews as handlers;
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

pub fn previews<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new()
		.post("/previews/:template", handlers::post_preview)
		.get("/previews/:template", handlers::get_preview)
		.get("/previews/:template/:id/video", handlers::get_video);

	Module::versioned("previews", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::POST, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION, RANGE])
}
