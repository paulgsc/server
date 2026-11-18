//! `/api/v1/feeds/:feed/videos` — each feed's videos on the phone, the
//! call's and Playoff Delta's alike (FEED4: paulgsc/some-ui#2647).
//!
//! ```text
//! GET    /feeds/:feed/videos                  → { episodes: [{ key, week, part, of, videoBytes, restBytes, link, receipts, title }] }: newest first
//! GET    /feeds/:feed/videos/:key/video       → the episode's video, streamed, with ranges
//! GET    /feeds/:feed/videos/:key/description → its `YouTube` chapter lines, `text/plain`, or 404
//! PUT    /feeds/:feed/videos/:key/link        → { url }: the body is { link }, a `YouTube` video's link as pasted
//! DELETE /feeds/:feed/videos/:key/video       → { receipt }: lets its video go, once a link is recorded
//! DELETE /feeds/:feed/videos/:key/rest        → { receipt }: lets the rest of its folder go, once the video has
//! ```
//!
//! `:feed` is `call`, `playoff-delta`, `rust-essay`, `interview-essay`,
//! `cards-essay` or `report` (`feeds::Feed`; an essay is the caller's own,
//! keyed by its week, and so is a week's engagement report, keyed by its ISO
//! week, `2026-W46`: contract epoch 17, #821); any other is `404`.
//!
//! **A cards essay** (EXPLAIN6, #878; contract epoch 10) is listed newest
//! day first, keyed `<slug>-<YYYY-MM-DD>`, with `title` from its cut list
//! (`null` when it has none, and for every other feed's episode), and is
//! done once `videoBytes` is set. Its video, description and link
//! (paulgsc/some-ui#2999, call C388) come through the routes above, as every
//! feed's. It is never let go: its `DELETE` routes are `404`, as for an
//! episode home doesn't hold.
//! `GET /cards-essays/:key/video` (`routes::cards_essays`) stays for the
//! Renders board.
//!
//! **A call's week that is a series of parts** (paulgsc/some-ui#2918) is
//! listed as each part, first to last, each an episode of its own: part 1
//! keyed by the week's day as a week of one video always was, part k by
//! `<day>-p<k>`, all sharing `week` and `of`. Each part is brought, linked
//! and let go on its own through the same routes, by its key.
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
