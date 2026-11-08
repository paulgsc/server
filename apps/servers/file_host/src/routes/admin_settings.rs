//! `/api/v1/admin/settings` — the operator's levers.
//!
//! What home follows while it runs, set from the web app instead of an
//! environment variable and a restart (paulgsc/server#501, ADMIN1;
//! paulgsc/some-ui#2357).
//!
//! ```text
//! GET /admin/settings/:namespace          → { namespace, value, isDefault, default, revision, setBy, setAt }
//! GET /admin/settings/:namespace/history  → { namespace, default, revisions: [{ revision, value, setBy, setAt }] },
//!                                           newest first, at most `HISTORY_CEILING`
//! PUT /admin/settings/:namespace          → { change, ...the setting as it now stands }: the body is the
//!                                           value; 422 with why its namespace refuses it, 404 for a
//!                                           namespace home does not have
//! ```
//!
//! The namespaces, their fields and their defaults are `settings`'
//! (`settings::NAMESPACES`): `rust-rounds` (`{ recording }`, on by default)
//! is the first. An unset setting reads as its default, with `isDefault`
//! true and no revision, who or when.
//!
//! **Trust model:** an operator only, as the lesson CRM's routes
//! (`curriculum_operator`): every handler takes `auth::operator::Operator`,
//! a browser's passkey session (`401` without one, the origin check on the
//! `PUT`, a paired phone's token `403`) of the owner's account or of a
//! subject `OPERATOR_SUBJECTS` lists (`403` for anyone else). The web app
//! puts its page in the `lan` build, which is a bundling choice and not an
//! access control.
//!
//! **Append-only.** A `PUT` adds a revision, or nothing when the value is the
//! one standing; there is no `DELETE`. Who set a revision is kept as a role
//! (`owner` or `operator`), never a subject (docs/identity.md invariant 37).

use crate::auth::AuthContext;
use crate::handlers::admin_settings as handlers;
use crate::routes::cors::allowlisted_cors_with_credentials;
use crate::routes::table::{Module, RouteTable};
use crate::Config;
use admin_setting_repo::BODY_CEILING;
use axum::{
	extract::{DefaultBodyLimit, FromRef},
	handler::Handler,
	http::{
		header::{ACCEPT, CONTENT_TYPE},
		Method,
	},
};
use sqlx::SqlitePool;
use tower_http::cors::CorsLayer;

pub fn admin_settings<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new()
		.get("/admin/settings/:namespace", handlers::get_setting)
		.get("/admin/settings/:namespace/history", handlers::get_history)
		// A value is at most `BODY_CEILING` bytes once re-serialised; the
		// limit refuses anything far past it before it is buffered.
		.put("/admin/settings/:namespace", handlers::put_setting.layer(DefaultBodyLimit::max(BODY_CEILING)));

	Module::versioned("admin_settings", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::PUT, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT])
}
