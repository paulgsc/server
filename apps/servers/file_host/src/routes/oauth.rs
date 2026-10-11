//! OAuth for AI services acting for a subject (docs/identity.md, "AI services
//! acting for a subject"). Three modules, by who calls them:
//!
//! ```text
//! GET    /.well-known/oauth-authorization-server     metadata (RFC 8414), unversioned
//! POST   /oauth/register                             a client registers (RFC 7591)
//! POST   /oauth/token                                a code or refresh token for tokens
//! POST   /oauth/authorize/requests                   the app hands in a request
//! POST   /oauth/authorize/requests/:request/approve  the signed-in subject approves
//! POST   /oauth/authorize/requests/:request/deny     ... or declines
//! GET    /oauth/grants                               connected services
//! DELETE /oauth/grants/:grant                        disconnect one
//! ```
//!
//! The metadata, registration and token routes are called server to server by
//! an AI service, through the public tunnel; they need no CORS. The rest are
//! the app's, on its own origin, with the credentialed allowlist every
//! subject-scoped module uses.

use crate::auth::AuthContext;
use crate::handlers::oauth as handlers;
use crate::routes::cors::allowlisted_cors_with_credentials;
use crate::routes::table::{Module, RouteTable};
use crate::Config;
use axum::{
	extract::FromRef,
	http::{header::CONTENT_TYPE, Method},
};
use tower_http::cors::CorsLayer;

/// Served at the root: RFC 8414 puts metadata under the issuer's own
/// `/.well-known/`, which must not move with the API version.
pub fn oauth_metadata<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	AuthContext: FromRef<S>,
{
	Module::unversioned("oauth_metadata", RouteTable::new().get("/.well-known/oauth-authorization-server", handlers::metadata))
}

/// What an AI service's servers call.
pub fn oauth<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new().post("/oauth/register", handlers::register).post("/oauth/token", handlers::token);
	Module::versioned("oauth", table)
}

/// What the app's approval page and Settings call.
pub fn oauth_approval<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new()
		.post("/oauth/authorize/requests", handlers::start_authorization)
		.post("/oauth/authorize/requests/:request/approve", handlers::approve)
		.post("/oauth/authorize/requests/:request/deny", handlers::deny)
		.get("/oauth/grants", handlers::grants)
		.delete("/oauth/grants/:grant", handlers::delete_grant);
	Module::versioned("oauth_approval", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::POST, Method::DELETE, Method::OPTIONS], vec![CONTENT_TYPE])
}
