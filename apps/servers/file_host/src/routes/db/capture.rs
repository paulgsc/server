use crate::handlers::db::capture as routes;
use crate::AppState;
use axum::{
	extract::FromRef,
	http::{
		header::{AUTHORIZATION, CONTENT_TYPE},
		HeaderValue, Method,
	},
	routing::{delete, get, post, put},
	Router,
};
use tower_http::cors::CorsLayer;

pub fn capture_sessions<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let cors = CorsLayer::new()
		.allow_origin("http://nixos.local:6006".parse::<HeaderValue>().unwrap())
		.allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
		.allow_headers([CONTENT_TYPE, AUTHORIZATION])
		.allow_credentials(true);

	Router::new()
		// ── Single ──────────────────────────────────────────────────────────
		// POST   /captures              → create one session
		// GET    /captures              → list all sessions (full payloads)
		// GET    /captures/:session_id  → get by client UUID
		// PUT    /captures/:session_id  → full replace
		// DELETE /captures/:session_id  → delete
		.route("/captures", post(routes::create_capture_session))
		.route("/captures", get(routes::get_all_capture_sessions))
		.route("/captures/:session_id", get(routes::get_capture_session))
		.route("/captures/:session_id", put(routes::update_capture_session))
		.route("/captures/:session_id", delete(routes::delete_capture_session))
		// ── Batch ───────────────────────────────────────────────────────────
		// POST   /captures/batch        → upsert multiple (ON CONFLICT UPDATE)
		// DELETE /captures/batch        → delete by session_id list
		.route("/captures/batch", post(routes::batch_create_capture_sessions))
		.route("/captures/batch", delete(routes::batch_delete_capture_sessions))
		// ── Query ───────────────────────────────────────────────────────────
		// GET    /captures/summaries          → lightweight CaptureSummary list
		// GET    /captures/date/:date         → filter by ISO-8601 date prefix
		.route("/captures/summaries", get(routes::get_capture_summaries))
		.route("/captures/date/:date", get(routes::get_capture_sessions_by_date))
		.layer(cors)
}
