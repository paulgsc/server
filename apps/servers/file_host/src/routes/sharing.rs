//! `/api/v1/sharing` — what a paired phone's person chose to keep sending
//! home, and when each choice began and ended (paulgsc/some-ui
//! `apps/www/src/lib/home-link`, its consent log).
//!
//! ```text
//! GET /sharing          → { kept, changes }: this pairing's choices
//! PUT /sharing/:scope   → the same, after { "kept": true | false }
//! ```
//!
//! **Trust model:** a paired phone's routes. Every handler takes a
//! `Paired`, so a request without a session is `401`, a write from an
//! untrusted origin `403`, and a browser's cookie session `403` too: the
//! phone's two-press confirm makes a choice, a signed-in browser never does.
//! Each choice is the pairing's alone.
//!
//! **Append-only.** A `PUT` adds a change or none; there is no `DELETE`.
//! Stopping only stops new data: what home holds stays. See
//! `handlers::sharing` and `sharing_repo`.

use crate::auth::AuthContext;
use crate::handlers::sharing as handlers;
use crate::routes::cors::allowlisted_cors_with_credentials;
use crate::routes::table::{Module, RouteTable};
use crate::Config;
use axum::{
	extract::FromRef,
	http::{
		header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE},
		Method,
	},
};
use sqlx::SqlitePool;
use tower_http::cors::CorsLayer;

pub fn sharing<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new().get("/sharing", handlers::get_choices).put("/sharing/:scope", handlers::put_choice);

	Module::versioned("sharing", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::PUT, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION])
}
