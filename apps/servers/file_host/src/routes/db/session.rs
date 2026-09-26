use crate::handlers::db::session as handlers;
use crate::routes::cors::allowlisted_cors;
use crate::routes::table::{Module, RouteTable};
use crate::{AppState, Config};
use axum::{
	extract::FromRef,
	http::{
		header::{AUTHORIZATION, CONTENT_TYPE},
		Method,
	},
};
use tower_http::cors::CorsLayer;

/// One route per `SessionsRepository` method, no more and no fewer.
///
/// Route order matters here: `/sessions/status` is declared before
/// `/sessions/:id` would swallow it. Axum matches literal segments ahead of
/// captures, so this is belt-and-braces rather than load-bearing, but the
/// grouping is the thing a reader checks first when a batch call starts
/// 404-ing with `"status"` as an id.
pub fn sessions<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let table = RouteTable::new()
		// ── Batch ───────────────────────────────────────────────────────────
		.patch("/sessions/status", handlers::set_status)
		// ── Collection ──────────────────────────────────────────────────────
		.get("/sessions", handlers::list_sessions)
		.post("/sessions", handlers::create_session)
		.delete("/sessions", handlers::delete_sessions)
		// ── Single ──────────────────────────────────────────────────────────
		.get("/sessions/:id", handlers::get_session)
		.patch("/sessions/:id", handlers::update_session)
		.delete("/sessions/:id", handlers::delete_session)
		.post("/sessions/:id/duplicate", handlers::duplicate_session);

	Module::versioned("sessions", table).with_cors(cors)
}

fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors(
		config,
		vec![Method::GET, Method::POST, Method::PATCH, Method::DELETE, Method::OPTIONS],
		vec![CONTENT_TYPE, AUTHORIZATION],
	)
}
