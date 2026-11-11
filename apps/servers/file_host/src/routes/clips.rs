//! `/api/v1/clips` — the clip frames home keeps, for the phone's Studio
//! (paulgsc/some-ui#2670's clips; the owner's decisions of 2026-11-11).
//!
//! ```text
//! GET /clips/:id/:side/:file    → the frame, image/png, ETag "<sha256>", immutable
//! ```
//!
//! **Trust model:** a paired phone's only, as `routes::feed_videos`'. A frame
//! is a screenshot of an app on sample data, but home serves it to no one
//! else: the handler takes a `Paired`, so a request without a session is
//! `401` and a browser's session `403`.
//!
//! What is kept is `clips`'s; the answers are `handlers::clips`'.

use crate::auth::AuthContext;
use crate::handlers::clips::{self as handlers, ClipFrames};
use crate::routes::cors::allowlisted_cors_with_credentials;
use crate::routes::table::{Module, RouteTable};
use crate::Config;
use axum::{
	extract::FromRef,
	http::{
		header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, IF_NONE_MATCH},
		Method,
	},
};
use sqlx::SqlitePool;
use tower_http::cors::CorsLayer;

pub fn clips<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
	ClipFrames: FromRef<S>,
{
	let table = RouteTable::new().get("/clips/:id/:side/:file", handlers::get_frame);
	Module::versioned("clips", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION, IF_NONE_MATCH])
}
