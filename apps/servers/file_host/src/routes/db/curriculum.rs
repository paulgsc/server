//! `/api/v1/curriculum` — lesson content, read-only (#276, CUR3).
//!
//! ```text
//! GET /curriculum/manifest   → { version, topiks: TopikMetadata[] }, bounded (curriculum_repo::MANIFEST_CEILING)
//! GET /curriculum/:key       → one lesson file, verbatim, or a JSON 404
//! ```
//!
//! What `@some-ui/topik` fetches from `/topiks/manifest.json` and
//! `/topiks/<key>.json` today, with one difference that is the point of the
//! route: a lesson that does not exist is a `404`, where a static file server
//! behind `try_files … /index.html` answers `200` with a page that happens to
//! parse as neither. The manifest's shape is transcribed from the client's
//! `TopikManifestSchema`, so its repository factories need a base path and
//! nothing else.
//!
//! No write route: lessons arrive through `import-curriculum` (#275). No
//! `SubjectId`: a lesson is corpus-wide, not owned by whoever studies it.
//! `ETag` is exposed and `If-None-Match` allowed cross-origin for the same
//! reason `routes::db::activities` gives.

use crate::handlers::db::curriculum as handlers;
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

pub fn curriculum<S>(config: &Config) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let cors = allowlisted_cors(config, vec![Method::GET, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT, IF_NONE_MATCH]).expose_headers([ETAG]);

	Router::new()
		.route("/curriculum/manifest", get(handlers::get_manifest))
		.route("/curriculum/:key", get(handlers::get_lesson))
		.layer(cors)
}
