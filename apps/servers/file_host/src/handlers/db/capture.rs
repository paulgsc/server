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

use capture_repo::{CaptureSessionRepository, StoredSession};
use ws_events::tabsched::{CaptureSession, CaptureSummary};

// ── Request/Response types ────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct BatchCreateRequest {
	pub sessions: Vec<CaptureSession>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct BatchDeleteRequest {
	pub session_ids: Vec<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct BatchDeleteResponse {
	pub deleted_count: u64,
}

// Row-level response that exposes the SQLite rowid alongside the session.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct StoredSessionResponse {
	pub id: i64,
	#[serde(flatten)]
	pub session: CaptureSession,
}

impl From<StoredSession> for StoredSessionResponse {
	fn from(s: StoredSession) -> Self {
		Self { id: s.rowid, session: s.session }
	}
}

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
	#[error("capture session not found")]
	NotFound,
	#[error("validation error: {0}")]
	ValidationError(String),
}

impl From<CaptureError> for FileHostError {
	fn from(err: CaptureError) -> Self {
		FileHostError::OperationError(err.to_string())
	}
}

// ── Cache key helpers ─────────────────────────────────────────────────────────

fn all_key() -> &'static str {
	"capture_sessions_all"
}

fn session_key(session_id: &str) -> String {
	format!("capture_session_{}", session_id)
}

fn summaries_key() -> &'static str {
	"capture_session_summaries"
}

fn date_key(date: &str) -> String {
	format!("capture_sessions_date_{}", date)
}

// ── Single handlers ───────────────────────────────────────────────────────────

#[axum::debug_handler]
#[instrument(
	name = "create_capture_session",
	skip_all,
	fields(
		otel.kind = "server",
		session_id = tracing::field::Empty
	)
)]
pub async fn create_capture_session(State(state): State<AppState>, Json(session): Json<CaptureSession>) -> Result<Json<StoredSessionResponse>, FileHostError> {
	tracing::Span::current().record("session_id", &session.session_id.as_str());
	let _timer = OperationTimer::new("create_capture_session", "total");

	let repo = Arc::new(CaptureSessionRepository::new(state.core.shared_db));

	let stored = {
		let _db_timer = OperationTimer::new("create_capture_session", "database_insert");
		repo.create(session).await
	}
	.map_err(|e| FileHostError::OperationError(e.to_string()))?;

	// Invalidate list/summary caches so the next read reflects the new row.
	{
		let _ct = OperationTimer::new("create_capture_session", "cache_invalidation");
		state.realtime.dedup_cache.delete(all_key()).await?;
		state.realtime.dedup_cache.delete(summaries_key()).await?;
		record_cache_invalidation("create_capture_session", 2);
	}

	Ok(Json(stored.into()))
}

#[axum::debug_handler]
#[instrument(
	name = "get_all_capture_sessions",
	skip_all,
	fields(otel.kind = "server")
)]
pub async fn get_all_capture_sessions(State(state): State<AppState>) -> Result<Json<Vec<CaptureSession>>, FileHostError> {
	let _timer = OperationTimer::new("get_all_capture_sessions", "total");

	let repo = Arc::new(CaptureSessionRepository::new(state.core.shared_db));

	let (sessions, was_cached) = state
		.realtime
		.dedup_cache
		.get_or_fetch(all_key(), || async {
			let _db = OperationTimer::new("get_all_capture_sessions", "database_fetch");
			repo.get_all().await.map_err(|e| DedupCacheError::OperationError(e.to_string()))
		})
		.await?;

	record_cache_hit("get_all_capture_sessions", was_cached);

	Ok(Json(sessions))
}

/// GET /captures/:session_id
///
/// `session_id` is the client-generated UUID, not the SQLite rowid.
/// This is the stable identifier the extension always knows.
#[axum::debug_handler]
#[instrument(
	name = "get_capture_session",
	skip_all,
	fields(
		otel.kind = "server",
		session_id = tracing::field::Empty
	)
)]
pub async fn get_capture_session(State(state): State<AppState>, Path(session_id): Path<String>) -> Result<Json<CaptureSession>, FileHostError> {
	tracing::Span::current().record("session_id", &session_id.as_str());
	let _timer = OperationTimer::new("get_capture_session", "total");

	let repo = Arc::new(CaptureSessionRepository::new(state.core.shared_db));
	let cache_key = session_key(&session_id);

	let (session_opt, was_cached) = state
		.realtime
		.dedup_cache
		.get_or_fetch(&cache_key, || async {
			let _db = OperationTimer::new("get_capture_session", "database_fetch");
			repo.get_by_session_id(&session_id).await.map_err(|e| DedupCacheError::OperationError(e.to_string()))
		})
		.await?;

	record_cache_hit("get_capture_session", was_cached);

	match session_opt {
		Some(s) => Ok(Json(s)),
		None => Err(CaptureError::NotFound.into()),
	}
}

/// PUT /captures/:session_id — full replace.
///
/// The extension re-sends the full `CaptureSession` payload; we overwrite the
/// stored row entirely and invalidate the affected cache keys.
#[axum::debug_handler]
#[instrument(
	name = "update_capture_session",
	skip_all,
	fields(
		otel.kind = "server",
		session_id = tracing::field::Empty
	)
)]
pub async fn update_capture_session(
	State(state): State<AppState>,
	Path(session_id): Path<String>,
	Json(session): Json<CaptureSession>,
) -> Result<Json<CaptureSession>, FileHostError> {
	tracing::Span::current().record("session_id", &session_id.as_str());
	let _timer = OperationTimer::new("update_capture_session", "total");

	let repo = Arc::new(CaptureSessionRepository::new(state.core.shared_db));

	let result_opt = {
		let _db = OperationTimer::new("update_capture_session", "database_update");
		repo.update_by_session_id(&session_id, session).await
	}
	.map_err(|e| FileHostError::OperationError(e.to_string()))?;

	let result = result_opt.ok_or(CaptureError::NotFound)?;

	{
		let _ct = OperationTimer::new("update_capture_session", "cache_invalidation");
		let cache_key = session_key(&session_id);
		state.realtime.dedup_cache.delete(&cache_key).await?;
		state.realtime.dedup_cache.delete(all_key()).await?;
		state.realtime.dedup_cache.delete(summaries_key()).await?;
		record_cache_invalidation("update_capture_session", 3);
	}

	Ok(Json(result))
}

#[axum::debug_handler]
#[instrument(
	name = "delete_capture_session",
	skip_all,
	fields(
		otel.kind = "server",
		session_id = tracing::field::Empty
	)
)]
pub async fn delete_capture_session(State(state): State<AppState>, Path(session_id): Path<String>) -> Result<StatusCode, FileHostError> {
	let _timer = OperationTimer::new("delete_capture_session", "total");

	let repo = Arc::new(CaptureSessionRepository::new(state.core.shared_db));

	let deleted = {
		let _db = OperationTimer::new("delete_capture_session", "database_delete");
		repo.delete_by_session_id(&session_id).await
	}
	.map_err(|e| FileHostError::OperationError(e.to_string()))?;

	if deleted {
		{
			let _ct = OperationTimer::new("delete_capture_session", "cache_invalidation");
			state.realtime.dedup_cache.delete(&session_key(&session_id)).await?;
			state.realtime.dedup_cache.delete(all_key()).await?;
			state.realtime.dedup_cache.delete(summaries_key()).await?;
			record_cache_invalidation("delete_capture_session", 3);
		}
		Ok(StatusCode::NO_CONTENT)
	} else {
		Err(CaptureError::NotFound.into())
	}
}

// ── Batch handlers ────────────────────────────────────────────────────────────

#[axum::debug_handler]
#[instrument(
	name = "batch_create_capture_sessions",
	skip_all,
	fields(
		otel.kind = "server",
		count = tracing::field::Empty
	)
)]
pub async fn batch_create_capture_sessions(State(state): State<AppState>, Json(request): Json<BatchCreateRequest>) -> Result<Json<Vec<CaptureSession>>, FileHostError> {
	tracing::Span::current().record("count", request.sessions.len());
	let _timer = OperationTimer::new("batch_create_capture_sessions", "total");

	if request.sessions.is_empty() {
		return Err(CaptureError::ValidationError("sessions list cannot be empty".into()).into());
	}

	let repo = Arc::new(CaptureSessionRepository::new(state.core.shared_db));

	let result = {
		let _db = OperationTimer::new("batch_create_capture_sessions", "database_batch_insert");
		repo.batch_create(request.sessions).await
	}
	.map_err(|e| FileHostError::OperationError(e.to_string()))?;

	{
		let _ct = OperationTimer::new("batch_create_capture_sessions", "cache_invalidation");
		state.realtime.dedup_cache.delete(all_key()).await?;
		state.realtime.dedup_cache.delete(summaries_key()).await?;
		record_cache_invalidation("batch_create_capture_sessions", 2);
	}

	Ok(Json(result))
}

#[axum::debug_handler]
#[instrument(
	name = "batch_delete_capture_sessions",
	skip_all,
	fields(
		otel.kind = "server",
		count = tracing::field::Empty
	)
)]
pub async fn batch_delete_capture_sessions(State(state): State<AppState>, Json(request): Json<BatchDeleteRequest>) -> Result<Json<BatchDeleteResponse>, FileHostError> {
	tracing::Span::current().record("count", request.session_ids.len());
	let _timer = OperationTimer::new("batch_delete_capture_sessions", "total");

	if request.session_ids.is_empty() {
		return Err(CaptureError::ValidationError("session_ids list cannot be empty".into()).into());
	}

	let repo = Arc::new(CaptureSessionRepository::new(state.core.shared_db));

	let deleted_count = {
		let _db = OperationTimer::new("batch_delete_capture_sessions", "database_batch_delete");
		repo.batch_delete(request.session_ids.clone()).await
	}
	.map_err(|e| FileHostError::OperationError(e.to_string()))?;

	{
		let _ct = OperationTimer::new("batch_delete_capture_sessions", "cache_invalidation");
		// individual session keys + all + summaries
		let n = request.session_ids.len() + 2;
		for sid in &request.session_ids {
			state.realtime.dedup_cache.delete(&session_key(sid)).await?;
		}
		state.realtime.dedup_cache.delete(all_key()).await?;
		state.realtime.dedup_cache.delete(summaries_key()).await?;
		record_cache_invalidation("batch_delete_capture_sessions", n);
	}

	Ok(Json(BatchDeleteResponse { deleted_count }))
}

// ── Query handlers ────────────────────────────────────────────────────────────

/// GET /captures/summaries — lightweight list without full tab payloads.
#[axum::debug_handler]
#[instrument(
	name = "get_capture_summaries",
	skip_all,
	fields(otel.kind = "server")
)]
pub async fn get_capture_summaries(State(state): State<AppState>) -> Result<Json<Vec<CaptureSummary>>, FileHostError> {
	let _timer = OperationTimer::new("get_capture_summaries", "total");

	let repo = Arc::new(CaptureSessionRepository::new(state.core.shared_db));

	let (summaries, was_cached) = state
		.realtime
		.dedup_cache
		.get_or_fetch(summaries_key(), || async {
			let _db = OperationTimer::new("get_capture_summaries", "database_fetch");
			repo.get_summaries().await.map_err(|e| DedupCacheError::OperationError(e.to_string()))
		})
		.await?;

	record_cache_hit("get_capture_summaries", was_cached);

	Ok(Json(summaries))
}

/// GET /captures/date/:date — ISO-8601 date prefix, e.g. "2025-04-17".
#[axum::debug_handler]
#[instrument(
	name = "get_capture_sessions_by_date",
	skip_all,
	fields(
		otel.kind = "server",
		date = tracing::field::Empty
	)
)]
pub async fn get_capture_sessions_by_date(State(state): State<AppState>, Path(date): Path<String>) -> Result<Json<Vec<CaptureSession>>, FileHostError> {
	tracing::Span::current().record("date", &date.as_str());
	let _timer = OperationTimer::new("get_capture_sessions_by_date", "total");

	let repo = Arc::new(CaptureSessionRepository::new(state.core.shared_db));
	let cache_key = date_key(&date);

	let (sessions, was_cached) = state
		.realtime
		.dedup_cache
		.get_or_fetch(&cache_key, || async {
			let _db = OperationTimer::new("get_capture_sessions_by_date", "database_fetch");
			repo.get_by_date(&date).await.map_err(|e| DedupCacheError::OperationError(e.to_string()))
		})
		.await?;

	record_cache_hit("get_capture_sessions_by_date", was_cached);

	Ok(Json(sessions))
}
