//! `GET /compat` (`handlers::compat`).
//!
//! Unversioned, like `/health`, since a client asks it before it knows which
//! API version home serves; CORS'd for any origin like `/health`, since the
//! web app asks it from the browser.

use crate::handlers::compat as routes;
use crate::routes::table::{Module, RouteTable};
use crate::{AppState, Config};
use axum::{extract::FromRef, http::Method};
use tower_http::cors::{Any, CorsLayer};

pub fn get_compat<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	Module::unversioned("compat", RouteTable::new().get("/compat", routes::compat)).with_cors(cors)
}

fn cors(_: &Config) -> CorsLayer {
	CorsLayer::new().allow_origin(Any).allow_methods([Method::GET]).allow_headers(Any)
}
