use crate::handlers::health as routes;
use crate::routes::table::{Module, RouteTable};
use crate::{AppState, Config};
use axum::{extract::FromRef, http::Method};
use tower_http::cors::{Any, CorsLayer};

pub fn get_health<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	Module::unversioned("health", RouteTable::new().get("/health", routes::health)).with_cors(cors)
}

fn cors(_: &Config) -> CorsLayer {
	CorsLayer::new()
		.allow_origin(Any) // Allow any origin (including extensions)
		.allow_methods([Method::GET])
		.allow_headers(Any)
}
