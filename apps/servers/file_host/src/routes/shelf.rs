//! `/api/v1/shelf` — the learner shelf (#387): content a learner generated
//! themselves and chose to keep, to replay it on another device.
//!
//! ```text
//! GET    /shelf/:activity        → { items: [{ key, contentHash, savedAt }], cap }
//! GET    /shelf/:activity/:key   → the kept body, verbatim (application/json)
//! PUT    /shelf/:activity/:key   → { change, item }: keep the request body itself
//! DELETE /shelf/:activity/:key   → 204, whether or not it was kept
//! ```
//!
//! `:activity` is `topik` or `leetype` (`learner_shelf_repo::Activity`).
//!
//! **Trust model:** a subject route. Every handler takes a `SubjectId`, so a
//! request without a passkey session is `401`, a state-changing one from an
//! untrusted origin is `403`, and every write holds off an account deletion
//! for the whole request. Every read and write is the session subject's own:
//! another subject's key is a `404` (or a `204` on delete), never a `403`, so
//! whether someone else kept something is not observable.
//!
//! Per person, never corpus: nothing here reads or writes `curriculum`,
//! `curriculum_publication` or `leetype_round`, so a kept item is never
//! served to anyone else or announced by the study nudge. See
//! `handlers::shelf` for the rest, and docs/study-nudge.md, "Learner shelf".

use crate::auth::AuthContext;
use crate::handlers::shelf as handlers;
use crate::routes::cors::allowlisted_cors_with_credentials;
use crate::routes::table::{Module, RouteTable};
use crate::Config;
use axum::{
	extract::FromRef,
	http::{
		header::{ACCEPT, CONTENT_TYPE},
		Method,
	},
};
use sqlx::SqlitePool;
use tower_http::cors::CorsLayer;

pub fn shelf<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new()
		.get("/shelf/:activity", handlers::list_items)
		.get("/shelf/:activity/:key", handlers::get_item)
		.put("/shelf/:activity/:key", handlers::put_item)
		.delete("/shelf/:activity/:key", handlers::delete_item);

	Module::versioned("shelf", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::PUT, Method::DELETE, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT])
}
