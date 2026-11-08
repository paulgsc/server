//! `/api/v1/renders` — what home is rendering, what waits for room, what
//! couldn't finish, what finished, and what's coming, for the phone's
//! Studio › Renders (SFEED13, paulgsc/some-ui#2761).
//!
//! ```text
//! GET /renders → { now, waiting, failed, done, coming }
//! ```
//!
//! **Trust model:** a paired phone's only, as `routes::feed_videos`'.
//!
//! What it reads is `feeds::renders`'s; the answer is `handlers::renders`'s.

use crate::auth::AuthContext;
use crate::handlers::renders as handlers;
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

pub fn renders<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	SqlitePool: FromRef<S>,
	AuthContext: FromRef<S>,
{
	let table = RouteTable::new().get("/renders", handlers::get_renders);
	Module::versioned("renders", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::OPTIONS], vec![CONTENT_TYPE, ACCEPT, AUTHORIZATION])
}
