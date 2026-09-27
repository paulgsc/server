use crate::auth::AuthContext;
use crate::handlers::auth as handlers;
use crate::routes::cors::allowlisted_cors;
use crate::routes::table::{Module, RouteTable};
use crate::Config;
use axum::{
	extract::FromRef,
	http::{header::CONTENT_TYPE, Method},
};
use tower_http::cors::CorsLayer;

/// Passkey auth (docs/identity.md, "Passkey auth"). Paths are relative:
/// `main.rs` nests this under `API_V1_BASE_PATH`.
///
/// Bounded on `AuthContext: FromRef<S>` alone, not `AppState`, so the whole
/// surface can be assembled and driven in a test with a pool and nothing else.
///
/// The session travels in a `SameSite=Strict` cookie, and the app reaches
/// these routes through its own origin's `/api/file-host` proxy, so no request
/// here is cross-origin in a deployment. CORS is the usual allowlist, with no
/// credentialed grant: a cross-origin caller gets no cookie to send anyway.
pub fn auth<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new()
		// A new account: create a passkey, and be signed in.
		.post("/auth/register/start", handlers::start_registration)
		.post("/auth/register/finish", handlers::finish_registration)
		// Sign in with any passkey the browser holds for this site.
		.post("/auth/sign-in/start", handlers::start_sign_in)
		.post("/auth/sign-in/finish", handlers::finish_sign_in)
		// Signed in: add a passkey on another device or ecosystem.
		.post("/auth/passkeys/start", handlers::start_adding_passkey)
		.post("/auth/passkeys/finish", handlers::finish_adding_passkey)
		// Is this browser signed in? Also where a session slides.
		.get("/auth/session", handlers::session)
		.post("/auth/sign-out", handlers::sign_out)
		.post("/auth/sign-out-everywhere", handlers::sign_out_everywhere)
		// Delete the account and every row stored for it.
		.delete("/auth/account", handlers::delete_account);

	Module::versioned("auth", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors(config, vec![Method::GET, Method::POST, Method::DELETE, Method::OPTIONS], vec![CONTENT_TYPE])
}
