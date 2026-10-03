//! The MCP endpoint and its RFC 9728 metadata (docs/identity.md, "The MCP
//! endpoint"). Two modules, by where they are served:
//!
//! ```text
//! GET  /.well-known/oauth-protected-resource/api/v1/mcp   who issues its tokens, unversioned
//! POST /mcp                                               JSON-RPC: initialize, ping, tools
//! ```
//!
//! An AI service's servers call both, through the public tunnel, with a
//! bearer token and no cookie, so neither has CORS: a browser page is not a
//! client of either. The metadata path is fixed because `OAUTH_RESOURCE` must
//! be this server's `/api/v1/mcp` (`auth::oauth::OAuthSettings::from_parts`).

use crate::auth::AuthContext;
use crate::handlers::mcp as handlers;
use crate::routes::table::{Module, RouteTable};
use axum::extract::FromRef;

/// Served at the root: RFC 9728 puts metadata under the origin's own
/// `/.well-known/`, followed by the resource's path.
pub fn mcp_metadata<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	AuthContext: FromRef<S>,
{
	Module::unversioned(
		"mcp_metadata",
		RouteTable::new().get("/.well-known/oauth-protected-resource/api/v1/mcp", handlers::resource_metadata),
	)
}

pub fn mcp<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	AuthContext: FromRef<S>,
{
	Module::versioned("mcp", RouteTable::new().post("/mcp", handlers::mcp))
}
