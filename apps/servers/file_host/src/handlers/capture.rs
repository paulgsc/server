use crate::{
	models::capture::{epoch_ms_score, index_insert, index_range, index_remove, session_cache_key},
	AppState, FileHostError,
};
use axum::{
	extract::{Path, Query, State},
	http::StatusCode,
	Json,
};
use serde::Deserialize;
use some_cache::DedupCacheError;
use tracing::{error, instrument};
use ws_events::tabsched::{CaptureSession, CaptureSummary, JobEnvelope};

// Precondition: body deserialises to CaptureSession; session_id non-empty.
// Postcondition: full session written to Redis via dedup cache;
//   session_id appended to sorted index scored by captured_at epoch ms.
//   Idempotent — re-POST of same session_id is a no-op on the cache write.
// Returns: CaptureSummary.
#[axum::debug_handler]
#[instrument(name = "post_capture", skip(state), fields(otel.kind = "server"))]
pub async fn post_capture(State(state): State<AppState>, Json(session): Json<CaptureSession>) -> Result<Json<CaptureSummary>, FileHostError> {
	let session_id = session.session_id.clone();
	let captured_at = session.captured_at.clone();
	let summary = CaptureSummary::from(&session);
	let key = session_cache_key(&session_id);
	let score = epoch_ms_score(&captured_at);

	// 1. Write to Redis (The Truth)
	state
		.realtime
		.dedup_cache
		.get_or_fetch(&key, || {
			let session = session.clone();
			async move { Ok::<CaptureSession, DedupCacheError>(session) }
		})
		.await?;

	index_insert(&state, &session.session_id, score).await?;

	// 2. Publish to JetStream (The Signal)
	let envelope = JobEnvelope {
		session_id: session_id.clone(),
		captured_at,
		attempt: 1,
	};

	// We publish to "pipeline.jobs" as defined in your PipelineSubjects
	state.realtime.pipeline_publisher.publish(&envelope).await.map_err(|e| {
		error!(%session_id, error = %e, "JetStream signaling failed");
		// We return an error to prevent the client from thinking
		// the background processing has started.
		e
	})?;

	Ok(Json(summary))
}

// Precondition: session_id path segment non-empty; key exists in Redis.
// Postcondition: returns full CaptureSession.
// Failure: DedupError::OperationError("not_found") → 404 via DedupError::into_response.
#[axum::debug_handler]
#[instrument(name = "get_capture", skip(state), fields(session_id = %session_id, otel.kind = "server"))]
pub async fn get_capture(State(state): State<AppState>, Path(session_id): Path<String>) -> Result<Json<CaptureSession>, FileHostError> {
	let key = session_cache_key(&session_id);

	let (session, _cached) = state
		.realtime
		.dedup_cache
		.get_or_fetch(&key, || async { Err(DedupCacheError::OperationError(format!("not_found: {}", key))) })
		.await?;

	Ok(Json(session))
}

#[derive(Debug, Deserialize)]
pub struct ListParams {
	pub from: Option<f64>,
	pub to: Option<f64>,
	pub limit: Option<usize>,
}

// Postcondition: returns Vec<CaptureSummary> ordered newest-first within [from, to].
// Note: summaries are reconstructed from full sessions; any key absent from Redis is silently skipped.
#[axum::debug_handler]
#[instrument(name = "list_captures", skip(state), fields(otel.kind = "server"))]
pub async fn list_captures(State(state): State<AppState>, Query(params): Query<ListParams>) -> Result<Json<Vec<CaptureSummary>>, FileHostError> {
	let limit = params.limit.unwrap_or(50).min(200);
	let from = params.from.unwrap_or(0.0);
	let to = params.to.unwrap_or(f64::MAX);

	let ids: Vec<String> = index_range(&state, from, to, limit).await?;

	let mut summaries = Vec::<CaptureSummary>::with_capacity(ids.len());
	for id in ids.iter().rev() {
		let key = session_cache_key(id);
		let fetch_result = state
			.realtime
			.dedup_cache
			.get_or_fetch::<CaptureSession, _, _>(&key, || async { Err(DedupCacheError::OperationError("not_found".into())) })
			.await;
		if let Ok((session, _)) = fetch_result {
			summaries.push(CaptureSummary::from(&session));
		}
	}

	Ok(Json(summaries))
}

// Precondition: session_id exists in Redis.
// Postcondition: key deleted from dedup cache; session_id removed from sorted index.
// Failure: 404 if key absent.
#[axum::debug_handler]
#[instrument(name = "delete_capture", skip(state), fields(session_id = %session_id, otel.kind = "server"))]
pub async fn delete_capture(State(state): State<AppState>, Path(session_id): Path<String>) -> Result<StatusCode, FileHostError> {
	let key = session_cache_key(&session_id);

	let deleted = state.realtime.dedup_cache.delete(&key).await?;
	if !deleted {
		return Err(DedupCacheError::OperationError(format!("not_found: {}", session_id)).into());
	}

	index_remove(&state, &session_id).await?;

	Ok(StatusCode::NO_CONTENT)
}
