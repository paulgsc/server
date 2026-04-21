use crate::metrics::otel::{record_cache_hit, record_cache_invalidation, OperationTimer};
use crate::{AppState, FileHostError};
use axum::{
	extract::{Path, State},
	http::StatusCode,
	Json,
};
use some_cache::DedupCacheError;
use std::sync::Arc;
use tracing::instrument;

use capture_repo::TabRepository;
use ws_events::tabsched::{TabCapture, TabSummary};

// ── Request/Response types ────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct BatchUpsertRequest {
	pub tabs: Vec<TabCapture>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct BatchUpsertResponse {
	pub upserted_count: u64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct BatchDeleteRequest {
	pub tab_ids: Vec<i64>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct BatchDeleteResponse {
	pub deleted_count: u64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct PruneRequest {
	/// Tabs not seen within this many days are deleted.
	/// Defaults to 30 if omitted.
	pub older_than_days: Option<i64>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct PruneResponse {
	pub pruned_count: u64,
}

/// Used by the extension to report which tab_ids are currently known.
/// Server returns the set that exists in the DB but was not reported —
/// candidates for explicit deletion.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ReconcileRequest {
	pub active_tab_ids: Vec<i64>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ReconcileResponse {
	/// tab_ids in DB that the extension did not report as active.
	pub absent_tab_ids: Vec<i64>,
}

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum TabError {
	#[error("tab not found")]
	NotFound,
	#[error("validation error: {0}")]
	ValidationError(String),
}

impl From<TabError> for FileHostError {
	fn from(err: TabError) -> Self {
		FileHostError::OperationError(err.to_string())
	}
}

// ── Cache key helpers ─────────────────────────────────────────────────────────

fn all_key() -> &'static str {
	"tabs_all"
}

fn tab_key(tab_id: i64) -> String {
	format!("tab_{}", tab_id)
}

fn summaries_key() -> &'static str {
	"tab_summaries"
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// POST /tabs — upsert a single tab.
/// Semantically identical to batch with one element; exists for convenience.
#[axum::debug_handler]
#[instrument(
	name = "upsert_tab",
	skip_all,
	fields(otel.kind = "server", tab_id = tracing::field::Empty)
)]
pub async fn upsert_tab(State(state): State<AppState>, Json(tab): Json<TabCapture>) -> Result<Json<TabCapture>, FileHostError> {
	tracing::Span::current().record("tab_id", tab.tab_id);
	let _timer = OperationTimer::new("upsert_tab", "total");

	let repo = Arc::new(TabRepository::new(state.core.shared_db));

	let result = {
		let _db = OperationTimer::new("upsert_tab", "database_upsert");
		repo.upsert(tab).await
	}
	.map_err(|e| FileHostError::OperationError(e.to_string()))?;

	{
		let _ct = OperationTimer::new("upsert_tab", "cache_invalidation");
		state.realtime.dedup_cache.delete(&tab_key(result.tab_id)).await?;
		state.realtime.dedup_cache.delete(all_key()).await?;
		state.realtime.dedup_cache.delete(summaries_key()).await?;
		record_cache_invalidation("upsert_tab", 3);
	}

	Ok(Json(result))
}

/// GET /tabs
#[axum::debug_handler]
#[instrument(name = "get_all_tabs", skip_all, fields(otel.kind = "server"))]
pub async fn get_all_tabs(State(state): State<AppState>) -> Result<Json<Vec<TabCapture>>, FileHostError> {
	let _timer = OperationTimer::new("get_all_tabs", "total");

	let repo = Arc::new(TabRepository::new(state.core.shared_db));

	let (tabs, was_cached) = state
		.realtime
		.dedup_cache
		.get_or_fetch(all_key(), || async {
			let _db = OperationTimer::new("get_all_tabs", "database_fetch");
			repo.get_all().await.map_err(|e| DedupCacheError::OperationError(e.to_string()))
		})
		.await?;

	record_cache_hit("get_all_tabs", was_cached);

	Ok(Json(tabs))
}

/// GET /tabs/:tab_id
#[axum::debug_handler]
#[instrument(
	name = "get_tab",
	skip_all,
	fields(otel.kind = "server", tab_id = tracing::field::Empty)
)]
pub async fn get_tab(State(state): State<AppState>, Path(tab_id): Path<i64>) -> Result<Json<TabCapture>, FileHostError> {
	tracing::Span::current().record("tab_id", tab_id);
	let _timer = OperationTimer::new("get_tab", "total");

	let repo = Arc::new(TabRepository::new(state.core.shared_db));
	let cache_key = tab_key(tab_id);

	let (tab_opt, was_cached) = state
		.realtime
		.dedup_cache
		.get_or_fetch(&cache_key, || async {
			let _db = OperationTimer::new("get_tab", "database_fetch");
			repo.get_by_tab_id(tab_id).await.map_err(|e| DedupCacheError::OperationError(e.to_string()))
		})
		.await?;

	record_cache_hit("get_tab", was_cached);

	match tab_opt {
		Some(t) => Ok(Json(t)),
		None => Err(TabError::NotFound.into()),
	}
}

/// DELETE /tabs/:tab_id — explicit close signal from extension.
#[axum::debug_handler]
#[instrument(
	name = "delete_tab",
	skip_all,
	fields(otel.kind = "server", tab_id = tracing::field::Empty)
)]
pub async fn delete_tab(State(state): State<AppState>, Path(tab_id): Path<i64>) -> Result<StatusCode, FileHostError> {
	tracing::Span::current().record("tab_id", tab_id);
	let _timer = OperationTimer::new("delete_tab", "total");

	let repo = Arc::new(TabRepository::new(state.core.shared_db));

	let deleted = {
		let _db = OperationTimer::new("delete_tab", "database_delete");
		repo.delete_by_tab_id(tab_id).await
	}
	.map_err(|e| FileHostError::OperationError(e.to_string()))?;

	if deleted {
		{
			let _ct = OperationTimer::new("delete_tab", "cache_invalidation");
			state.realtime.dedup_cache.delete(&tab_key(tab_id)).await?;
			state.realtime.dedup_cache.delete(all_key()).await?;
			state.realtime.dedup_cache.delete(summaries_key()).await?;
			record_cache_invalidation("delete_tab", 3);
		}
		Ok(StatusCode::NO_CONTENT)
	} else {
		Err(TabError::NotFound.into())
	}
}

// ── Batch ─────────────────────────────────────────────────────────────────────

/// POST /tabs/batch — primary write path from extension.
/// Upserts all tabs in a single transaction.
#[axum::debug_handler]
#[instrument(
	name = "batch_upsert_tabs",
	skip_all,
	fields(otel.kind = "server", count = tracing::field::Empty)
)]
pub async fn batch_upsert_tabs(State(state): State<AppState>, Json(request): Json<BatchUpsertRequest>) -> Result<Json<BatchUpsertResponse>, FileHostError> {
	tracing::Span::current().record("count", request.tabs.len());
	let _timer = OperationTimer::new("batch_upsert_tabs", "total");

	if request.tabs.is_empty() {
		return Err(TabError::ValidationError("tabs list cannot be empty".into()).into());
	}

	let repo = Arc::new(TabRepository::new(state.core.shared_db));

	// Invalidate per-tab keys before the write so stale reads can't race.
	{
		let _ct = OperationTimer::new("batch_upsert_tabs", "cache_pre_invalidation");
		for tab in &request.tabs {
			state.realtime.dedup_cache.delete(&tab_key(tab.tab_id)).await?;
		}
	}

	let upserted_count = {
		let _db = OperationTimer::new("batch_upsert_tabs", "database_batch_upsert");
		repo.batch_upsert(request.tabs).await
	}
	.map_err(|e| FileHostError::OperationError(e.to_string()))?;

	{
		let _ct = OperationTimer::new("batch_upsert_tabs", "cache_invalidation");
		state.realtime.dedup_cache.delete(all_key()).await?;
		state.realtime.dedup_cache.delete(summaries_key()).await?;
		record_cache_invalidation("batch_upsert_tabs", 2);
	}

	Ok(Json(BatchUpsertResponse { upserted_count }))
}

/// DELETE /tabs/batch
#[axum::debug_handler]
#[instrument(
	name = "batch_delete_tabs",
	skip_all,
	fields(otel.kind = "server", count = tracing::field::Empty)
)]
pub async fn batch_delete_tabs(State(state): State<AppState>, Json(request): Json<BatchDeleteRequest>) -> Result<Json<BatchDeleteResponse>, FileHostError> {
	tracing::Span::current().record("count", request.tab_ids.len());
	let _timer = OperationTimer::new("batch_delete_tabs", "total");

	if request.tab_ids.is_empty() {
		return Err(TabError::ValidationError("tab_ids list cannot be empty".into()).into());
	}

	let repo = Arc::new(TabRepository::new(state.core.shared_db));

	let deleted_count = {
		let _db = OperationTimer::new("batch_delete_tabs", "database_batch_delete");
		repo.batch_delete(request.tab_ids.clone()).await
	}
	.map_err(|e| FileHostError::OperationError(e.to_string()))?;

	{
		let _ct = OperationTimer::new("batch_delete_tabs", "cache_invalidation");
		let n = request.tab_ids.len() + 2;
		for id in &request.tab_ids {
			state.realtime.dedup_cache.delete(&tab_key(*id)).await?;
		}
		state.realtime.dedup_cache.delete(all_key()).await?;
		state.realtime.dedup_cache.delete(summaries_key()).await?;
		record_cache_invalidation("batch_delete_tabs", n);
	}

	Ok(Json(BatchDeleteResponse { deleted_count }))
}

// ── Maintenance ───────────────────────────────────────────────────────────────

/// POST /tabs/prune — time-based hard delete of stale tabs.
/// Called by a periodic job or manually. TTL defaults to 30 days.
#[axum::debug_handler]
#[instrument(name = "prune_tabs", skip_all, fields(otel.kind = "server"))]
pub async fn prune_tabs(State(state): State<AppState>, Json(request): Json<PruneRequest>) -> Result<Json<PruneResponse>, FileHostError> {
	let _timer = OperationTimer::new("prune_tabs", "total");

	let older_than_days = request.older_than_days.unwrap_or(30);
	if older_than_days < 1 {
		return Err(TabError::ValidationError("older_than_days must be >= 1".into()).into());
	}

	let repo = Arc::new(TabRepository::new(state.core.shared_db));

	let pruned_count = {
		let _db = OperationTimer::new("prune_tabs", "database_delete");
		repo.prune_stale(older_than_days).await
	}
	.map_err(|e| FileHostError::OperationError(e.to_string()))?;

	if pruned_count > 0 {
		// Stale cache can't know which tab keys to drop; nuke list caches.
		state.realtime.dedup_cache.delete(all_key()).await?;
		state.realtime.dedup_cache.delete(summaries_key()).await?;
		record_cache_invalidation("prune_tabs", 2);
	}

	Ok(Json(PruneResponse { pruned_count }))
}

/// POST /tabs/reconcile — extension reports currently known tab_ids;
/// server returns ids that exist in DB but were not reported.
/// Caller decides whether to delete them.
#[axum::debug_handler]
#[instrument(name = "reconcile_tabs", skip_all, fields(otel.kind = "server"))]
pub async fn reconcile_tabs(State(state): State<AppState>, Json(request): Json<ReconcileRequest>) -> Result<Json<ReconcileResponse>, FileHostError> {
	let _timer = OperationTimer::new("reconcile_tabs", "total");

	let repo = Arc::new(TabRepository::new(state.core.shared_db));

	let absent_tab_ids = repo.find_absent(&request.active_tab_ids).await.map_err(|e| FileHostError::OperationError(e.to_string()))?;

	Ok(Json(ReconcileResponse { absent_tab_ids }))
}

// ── Query ─────────────────────────────────────────────────────────────────────

/// GET /tabs/summaries — lightweight; no content blobs.
#[axum::debug_handler]
#[instrument(name = "get_tab_summaries", skip_all, fields(otel.kind = "server"))]
pub async fn get_tab_summaries(State(state): State<AppState>) -> Result<Json<Vec<TabSummary>>, FileHostError> {
	let _timer = OperationTimer::new("get_tab_summaries", "total");

	let repo = Arc::new(TabRepository::new(state.core.shared_db));

	let (summaries, was_cached) = state
		.realtime
		.dedup_cache
		.get_or_fetch(summaries_key(), || async {
			let _db = OperationTimer::new("get_tab_summaries", "database_fetch");
			repo.get_summaries().await.map_err(|e| DedupCacheError::OperationError(e.to_string()))
		})
		.await?;

	record_cache_hit("get_tab_summaries", was_cached);

	Ok(Json(summaries))
}
