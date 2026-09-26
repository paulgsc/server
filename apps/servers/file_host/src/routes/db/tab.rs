use crate::handlers::db::tab as routes;
use crate::routes::cors::allowlisted_cors_with_credentials;
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

pub fn tabs<S>() -> Module<S>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	let table = RouteTable::new()
		// ── Single ──────────────────────────────────────────────────────────
		// POST   /tabs            → upsert one tab (create or update)
		// GET    /tabs            → all tabs (full payloads)
		// GET    /tabs/:tab_id    → single tab by browser tab_id
		// DELETE /tabs/:tab_id    → explicit close (from tabs.onRemoved)
		.post("/tabs", routes::upsert_tab)
		.get("/tabs", routes::get_all_tabs)
		.get("/tabs/:tab_id", routes::get_tab)
		.delete("/tabs/:tab_id", routes::delete_tab)
		// ── Batch ───────────────────────────────────────────────────────────
		// POST   /tabs/batch      → primary write path; upsert Vec<TabCapture>
		// DELETE /tabs/batch      → delete by tab_id list
		.post("/tabs/batch", routes::batch_upsert_tabs)
		.delete("/tabs/batch", routes::batch_delete_tabs)
		// ── Maintenance ─────────────────────────────────────────────────────
		// POST   /tabs/prune      → hard-delete tabs stale beyond TTL
		// POST   /tabs/reconcile  → diff active ids against DB; returns absent
		.post("/tabs/prune", routes::prune_tabs)
		.post("/tabs/reconcile", routes::reconcile_tabs)
		// ── Query ───────────────────────────────────────────────────────────
		// GET    /tabs/summaries  → lightweight TabSummary list (no blobs)
		.get("/tabs/summaries", routes::get_tab_summaries)
		// ── Pipeline ─────────────────────────────────────────────────────────
		// POST    /tabs/pipeline    → Trigger NATS job for offline processing
		.post("/tabs/pipeline", routes::trigger_pipeline);

	Module::versioned("tabs", table).with_cors(cors)
}

// Was a hardcoded `http://nixos.local:6006` — Storybook's port, not the
// app's. Now the same `ALLOWED_ORIGINS` list every other browser-facing
// route uses.
fn cors(config: &Config) -> CorsLayer {
	allowlisted_cors_with_credentials(config, vec![Method::GET, Method::POST, Method::DELETE], vec![CONTENT_TYPE, AUTHORIZATION])
}
