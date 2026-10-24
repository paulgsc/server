//! `/api/v1/routine` — what the owner's cloud routines left for the phone
//! through the Jobs mailbox, other than the Jobs digests (`routes::jobs`).
//!
//! ```text
//! GET /routine/files → { files }: the reflections routine's newest word
//!                      sets and dramas home holds, newest first, at most 20
//! ```
//!
//! **Trust model:** a subject route, as `routes::jobs`: a request without a
//! session is `401`. Read only: the `mailbox` poller is the files' one
//! writer (docs/identity.md invariant 19).

use crate::auth::AuthContext;
use crate::handlers::routine as handlers;
use crate::routes::cors::allowlisted_cors_with_credentials;
use crate::routes::table::{Module, RouteTable};
use crate::Config;
use axum::{
	extract::FromRef,
	http::{
		header::{ACCEPT, AUTHORIZATION},
		Method,
	},
};
use sqlx::SqlitePool;
use tower_http::cors::CorsLayer;

pub fn routine<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new().get("/routine/files", handlers::get_files);
	Module::versioned("routine", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::OPTIONS], vec![ACCEPT, AUTHORIZATION])
}
