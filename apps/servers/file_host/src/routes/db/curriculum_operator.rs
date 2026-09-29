//! `/api/v1/curriculum/operator` — the operator's hand on the lessons the
//! server serves (the LAN lesson CRM, `paulgsc/some-ui`).
//!
//! ```text
//! GET  /curriculum/operator/lessons               → { lessons: OperatorLesson[] }, retired included, bounded
//! PUT  /curriculum/operator/lessons/:key          → { change, lesson }: write one lesson, `{ activityId, metadata, body }`
//! POST /curriculum/operator/lessons/:key/retire   → the lesson, out of the manifest
//! POST /curriculum/operator/lessons/:key/restore  → the lesson, back in it
//! ```
//!
//! `import-curriculum` (#275) stays the way to bring a corpus across in bulk;
//! these routes are the way to change one lesson without a shell. Both write
//! through `CurriculumRepository::upsert`, so a lesson's content hash, version
//! and `published_at` mean the same thing whichever way it arrived, and the
//! waker announces what either one publishes (#277). The manifest serves the
//! **listed** lessons, so the listed set is the operator's weekly batch:
//! retiring takes a lesson out of it without deleting it, since a learner's
//! resume point or survey may still name it (`GET /curriculum/:key` still
//! serves it). Neither retiring nor restoring is a publication.
//!
//! The paths sit a segment below `/curriculum/:key` so they shadow no lesson
//! key, the way `/curriculum/manifest` shadows `manifest`.
//!
//! **Trust model:** `file_host`'s own, and no more — the CORS origin allowlist,
//! no authentication (`docs/study-nudge.md`, "Trust model, stated plainly").
//! Anyone who can reach the LAN can rewrite the lessons everyone is served.
//! The client puts its CRM in a LAN-only build, which is a bundling choice and
//! not an access control. No `SubjectId`: a lesson is corpus-wide.
//!
//! The server stays blind to what a lesson is: `body` is stored verbatim and
//! checked only for being JSON under `curriculum_repo::LESSON_BYTES_CEILING`.
//! The client checks the lesson itself before it writes (`intakeLesson`).

use crate::handlers::db::curriculum_operator as handlers;
use crate::routes::cors::allowlisted_cors_with_credentials;
use crate::routes::table::{Module, RouteTable};
use crate::{AppState, Config};
use axum::{
	extract::FromRef,
	http::{
		header::{ACCEPT, CONTENT_TYPE},
		Method,
	},
};
use tower_http::cors::CorsLayer;

pub fn curriculum_operator<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let table = RouteTable::new()
		.get("/curriculum/operator/lessons", handlers::get_lessons)
		.put("/curriculum/operator/lessons/:key", handlers::put_lesson)
		.post("/curriculum/operator/lessons/:key/retire", handlers::retire_lesson)
		.post("/curriculum/operator/lessons/:key/restore", handlers::restore_lesson);

	Module::versioned("curriculum_operator", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::PUT, Method::POST, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT])
}
