//! `/api/v1/feeds/:feed/videos` — each feed's videos on the phone, the
//! call's and Playoff Delta's alike (FEED4: paulgsc/some-ui#2647).
//!
//! ```text
//! GET    /feeds/:feed/videos                  → { episodes: [{ key, videoBytes, restBytes, link, receipts }] }: newest first
//! GET    /feeds/:feed/videos/:key/video       → the episode's video, streamed, with ranges
//! GET    /feeds/:feed/videos/:key/description → its `YouTube` chapter lines, `text/plain`, or 404
//! PUT    /feeds/:feed/videos/:key/link        → { url }: the body is { link }, a `YouTube` video's link as pasted
//! DELETE /feeds/:feed/videos/:key/video       → { receipt }: lets its video go, once a link is recorded
//! DELETE /feeds/:feed/videos/:key/rest        → { receipt }: lets the rest of its folder go, once the video has
//! ```
//!
//! `:feed` is `call` or `playoff-delta` (`feeds::Feed`); any other is `404`.
//!
//! **Trust model:** a paired phone's only, as `routes::voice`'s delete. Every
//! handler takes a `Paired`, so a request without a session is `401` and a
//! browser's session `403`.
//!
//! The record's rules are each feed's repository's; what goes on the drive
//! is `feeds::videos`'s; the answers are `handlers::feed_videos`'s.

use crate::auth::AuthContext;
use crate::handlers::feed_videos as handlers;
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

pub fn feed_videos<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new()
		.get("/feeds/:feed/videos", handlers::get_episodes)
		.get("/feeds/:feed/videos/:key/video", handlers::get_video)
		.get("/feeds/:feed/videos/:key/description", handlers::get_description)
		.put("/feeds/:feed/videos/:key/link", handlers::put_link)
		.delete("/feeds/:feed/videos/:key/video", handlers::delete_video)
		.delete("/feeds/:feed/videos/:key/rest", handlers::delete_rest);

	Module::versioned("feed_videos", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(
		config,
		vec![Method::GET, Method::PUT, Method::DELETE, Method::OPTIONS],
		vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION, RANGE],
	)
}
