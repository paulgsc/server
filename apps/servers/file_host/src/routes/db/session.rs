use crate::handlers::db::session as handlers;
use crate::routes::cors::allowlisted_cors;
use crate::{AppState, Config};
use axum::{
	extract::FromRef,
	http::{
		header::{AUTHORIZATION, CONTENT_TYPE},
		Method,
	},
	routing::{delete, get, patch, post},
	Router,
};

/// One route per `SessionsRepository` method, no more and no fewer.
///
/// Route order matters here: `/sessions/status` is declared before
/// `/sessions/:id` would swallow it. Axum matches literal segments ahead of
/// captures, so this is belt-and-braces rather than load-bearing, but the
/// grouping is the thing a reader checks first when a batch call starts
/// 404-ing with `"status"` as an id.
pub fn sessions<S>(config: &Config) -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let cors = allowlisted_cors(
		config,
		vec![Method::GET, Method::POST, Method::PATCH, Method::DELETE, Method::OPTIONS],
		vec![CONTENT_TYPE, AUTHORIZATION],
	);

	Router::new()
		// ── Batch ───────────────────────────────────────────────────────────
		.route("/sessions/status", patch(handlers::set_status))
		// ── Collection ──────────────────────────────────────────────────────
		.route("/sessions", get(handlers::list_sessions))
		.route("/sessions", post(handlers::create_session))
		.route("/sessions", delete(handlers::delete_sessions))
		// ── Single ──────────────────────────────────────────────────────────
		.route("/sessions/:id", get(handlers::get_session))
		.route("/sessions/:id", patch(handlers::update_session))
		.route("/sessions/:id", delete(handlers::delete_session))
		.route("/sessions/:id/duplicate", post(handlers::duplicate_session))
		.layer(cors)
}
