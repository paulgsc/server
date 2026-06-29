use crate::handlers::db::tab as routes;
use crate::AppState;
use axum::{
	extract::FromRef,
	http::{
		header::{AUTHORIZATION, CONTENT_TYPE},
		HeaderValue, Method,
	},
	routing::{delete, get, post},
	Router,
};
use tower_http::cors::CorsLayer;

pub fn tabs<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let cors = CorsLayer::new()
		.allow_origin("http://nixos.local:6006".parse::<HeaderValue>().unwrap())
		.allow_methods([Method::GET, Method::POST, Method::DELETE])
		.allow_headers([CONTENT_TYPE, AUTHORIZATION])
		.allow_credentials(true);

	Router::new()
		// ── Single ──────────────────────────────────────────────────────────
		// POST   /tabs            → upsert one tab (create or update)
		// GET    /tabs            → all tabs (full payloads)
		// GET    /tabs/:tab_id    → single tab by browser tab_id
		// DELETE /tabs/:tab_id    → explicit close (from tabs.onRemoved)
		.route("/tabs", post(routes::upsert_tab))
		.route("/tabs", get(routes::get_all_tabs))
		.route("/tabs/:tab_id", get(routes::get_tab))
		.route("/tabs/:tab_id", delete(routes::delete_tab))
		// ── Batch ───────────────────────────────────────────────────────────
		// POST   /tabs/batch      → primary write path; upsert Vec<TabCapture>
		// DELETE /tabs/batch      → delete by tab_id list
		.route("/tabs/batch", post(routes::batch_upsert_tabs))
		.route("/tabs/batch", delete(routes::batch_delete_tabs))
		// ── Maintenance ─────────────────────────────────────────────────────
		// POST   /tabs/prune      → hard-delete tabs stale beyond TTL
		// POST   /tabs/reconcile  → diff active ids against DB; returns absent
		.route("/tabs/prune", post(routes::prune_tabs))
		.route("/tabs/reconcile", post(routes::reconcile_tabs))
		// ── Query ───────────────────────────────────────────────────────────
		// GET    /tabs/summaries  → lightweight TabSummary list (no blobs)
		.route("/tabs/summaries", get(routes::get_tab_summaries))
		// ── Pipeline ─────────────────────────────────────────────────────────
		// POST    /tabs/pipeline    → Trigger NATS job for offline processing
		.route("/tabs/pipeline", post(routes::trigger_pipeline))
		.layer(cors)
}
