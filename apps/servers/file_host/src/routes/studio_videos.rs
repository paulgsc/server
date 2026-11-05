//! `/api/v1/studio/videos` — finished Studio videos, queued on home as their
//! DNA.
//!
//! From STUDIO ST33, slice 1 (paulgsc/some-ui#2591; the owner's decision 9
//! on paulgsc/some-ui#2248): finishing a video on the phone files a promise,
//! not a session. Its DNA (the template and version it goes through, the
//! shape built in Studio, the takes by voice note id, and the data that
//! fills it) goes home as one record per revision, and Studio reads back
//! where each one stands.
//!
//! ```text
//! GET /studio/videos                     → { videos: [{ video, rev, title, queuedAt, storedAt, stage }] }: newest queued first, at most 200
//! PUT /studio/videos/:video/queued/:rev  → { change, video }: the body is the DNA, `some-ui.video-dna/1`, at most 256 KiB
//! ```
//!
//! `stage` is `queued` in this slice. Slice 2 adds the routine that fills a
//! DNA (`filling`, `filled`, `problem`) and files it for the mailbox; slice 3
//! adds the render (`made`). Until slice 2 amends docs/identity.md invariant
//! 35, nothing reads a queued DNA for a routine or the mailbox.
//!
//! **Trust model:** a subject route, as `routes::ideas`. Every handler takes
//! a `SubjectId`, so a request without a session is `401` and a write from an
//! untrusted origin `403`. The phone reaches it with its pairing's bearer
//! token.
//!
//! **Append-only.** A `PUT` adds a revision or changes nothing, and other
//! bytes under a queued revision are a `409`; there is no `DELETE`. What a DNA
//! may hold is `studio_video_repo`'s. See `handlers::studio_videos`.

use crate::auth::AuthContext;
use crate::handlers::studio_videos as handlers;
use crate::routes::cors::allowlisted_cors_with_credentials;
use crate::routes::table::{Module, RouteTable};
use crate::Config;
use axum::{
	extract::{DefaultBodyLimit, FromRef},
	handler::Handler,
	http::{
		header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE},
		Method,
	},
};
use sqlx::SqlitePool;
use studio_video_repo::DNA_CEILING;
use tower_http::cors::CorsLayer;

pub fn studio_videos<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new()
		.get("/studio/videos", handlers::get_videos)
		// A DNA is at most `DNA_CEILING` bytes. The limit is one past it, so a
		// body of exactly one byte too many still gets the 422 that names the
		// ceiling, and anything longer a 413 before it is buffered, well inside
		// the server-wide `RequestBodyLimitLayer`.
		.put("/studio/videos/:video/queued/:rev", handlers::put_queued.layer(DefaultBodyLimit::max(DNA_CEILING + 1)));

	Module::versioned("studio_videos", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::PUT, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION])
}
