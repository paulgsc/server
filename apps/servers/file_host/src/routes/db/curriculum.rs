//! `/api/v1/curriculum` — lesson content, read-only (#276, CUR3).
//!
//! ```text
//! GET /curriculum/manifest        → { version, topiks: TopikMetadata[] }, bounded (curriculum_repo::MANIFEST_CEILING)
//! GET /curriculum/manifest.json   → the same manifest
//! GET /curriculum/:key            → one lesson file, verbatim, or a JSON 404; `<key>.json` works too
//! ```
//!
//! What `@some-ui/topik` fetches from `/topiks/manifest.json` and
//! `/topiks/<key>.json` today, with one difference that is the point of the
//! route: a lesson that does not exist is a `404`, where a static file server
//! behind `try_files … /index.html` answers `200` with a page that happens to
//! parse as neither. The manifest's shape is transcribed from the client's
//! `TopikManifestSchema`, and both the client's `.json` file names and the
//! bare names are answered, so its repository factories need a base path and
//! nothing else — `apps/www` builds `${root}/manifest.json` and
//! `${root}/<key>.json` (a real `chatgpt-codex-connector` finding on #366).
//! The static `manifest.json` route wins over `:key` in axum's router, so
//! the manifest is never looked up as a lesson key.
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
		.route("/curriculum/manifest.json", get(handlers::get_manifest))
		.route("/curriculum/:key", get(handlers::get_lesson))
		.layer(cors)
}
