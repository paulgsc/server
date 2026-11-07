//! `/api/v1/call/videos` — the weekly call's videos on the phone (STUDIO
//! ST38: paulgsc/some-ui#2661).
//!
//! ```text
//! GET    /call/videos              → { weeks: [{ from, to, videoBytes, restBytes, link, receipts }] }: newest first
//! GET    /call/videos/:from/video  → the week's call.mp4, streamed, with ranges
//! PUT    /call/videos/:from/link   → { url }: the body is { link }, a `YouTube` video's link as pasted
//! DELETE /call/videos/:from/video  → { receipt }: lets its call.mp4 go, once a link is recorded
//! DELETE /call/videos/:from/rest   → { receipt }: lets the rest of its folder go, once the video has
//! ```
//!
//! **Trust model:** a paired phone's only, as `routes::voice`'s delete. Every
//! handler takes a `Paired`, so a request without a session is `401` and a
//! browser's session `403`.
//!
//! The record's rules are `call_week_repo::video`'s; what goes on the drive
//! is `call_weeks::videos`'s; the answers are `handlers::call_videos`'s.

use crate::auth::AuthContext;
use crate::handlers::call_videos as handlers;
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

pub fn call_videos<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new()
		.get("/call/videos", handlers::get_weeks)
		.get("/call/videos/:from/video", handlers::get_video)
		.put("/call/videos/:from/link", handlers::put_link)
		.delete("/call/videos/:from/video", handlers::delete_video)
		.delete("/call/videos/:from/rest", handlers::delete_rest);

	Module::versioned("call_videos", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(
		config,
		vec![Method::GET, Method::PUT, Method::DELETE, Method::OPTIONS],
		vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION, RANGE],
	)
}
