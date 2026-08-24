//! `/api/v1/activities` — the server-owned catalogue, read-only. #271.
//!
//! ```text
//! GET /activities        → the full catalogue, bounded (see activity_repo::CATALOG_CEILING)
//! GET /activities/:id    → one activity, or 404
//! ```
//!
//! No write route: the catalogue is seeded and migrated, not `POSTed` — #271's
//! own out-of-scope note. No `SubjectId` extractor either, unlike
//! `routes/db/session.rs`'s surface: an activity is catalogue-wide, not
//! owned by whoever plays it (see `activity_repo`'s crate doc), so there is
//! no subject to scope this by.
//!
//! `ETag` is exposed cross-origin deliberately: browsers only expose the
//! CORS "simple response header" set by default, and `ETag` is not in it, so
//! a client's `fetch` could never read it without `expose_headers`. The
//! client also has to be allowed to *send* `If-None-Match`, which is why
//! that header appears in `allow_headers` as well as `expose_headers`.

use crate::handlers::db::activities as handlers;
use crate::routes::cors::allowlisted_cors;
use crate::{AppState, Config};
use axum::{
	extract::FromRef,
	http::{
		header::{ACCEPT, CONTENT_TYPE, ETAG, IF_NONE_MATCH},
		Method,
	},
	routing::get,
	Router,
};

pub fn activities<S>(config: &Config) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let cors = allowlisted_cors(config, vec![Method::GET, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT, IF_NONE_MATCH]).expose_headers([ETAG]);

	Router::new()
		.route("/activities", get(handlers::list_activities))
		.route("/activities/:id", get(handlers::get_activity))
		.layer(cors)
}
