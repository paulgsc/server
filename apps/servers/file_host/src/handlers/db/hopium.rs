use crate::metrics::otel::{record_cache_hit, record_cache_invalidation, OperationTimer};
use crate::{AppState, DedupError};
use axum::{
	extract::{Path, State},
	http::StatusCode,
	Json,
};
use mood_event::core::{CreateMoodEvent, MoodEvent, MoodEventRepository, MoodStats, UpdateMoodEvent};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tracing::instrument;

// Request/Response types for batch operations
#[derive(Debug, Serialize, Deserialize)]
pub struct BatchCreateRequest {
	pub events: Vec<CreateMoodEvent>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BatchUpdateRequest {
	pub updates: HashMap<i64, UpdateMoodEvent>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BatchDeleteRequest {
	pub ids: Vec<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BatchDeleteResponse {
	pub deleted_count: u64,
}

// Define a proper error type for MoodEvent operations
#[derive(Debug, thiserror::Error)]
pub enum MoodEventError {
	#[error("Mood event not found")]
	NotFound,
	#[error("Validation error: {0}")]
	ValidationError(String),
}

impl From<MoodEventError> for DedupError {
	fn from(err: MoodEventError) -> Self {
		DedupError::OperationError(err.to_string())
	}
}

// Single mood event handlers
#[axum::debug_handler]
#[instrument(name = "create_mood_event", skip(state), fields(otel.kind = "server"))]
pub async fn create_mood_event(State(state): State<AppState>, Json(event): Json<CreateMoodEvent>) -> Result<Json<MoodEvent>, DedupError> {
	let _timer = OperationTimer::new("create_mood_event", "total");

	let mood_repository = Arc::new(MoodEventRepository::new(state.core.shared_db));

	let result = {
		let _db_timer = OperationTimer::new("create_mood_event", "database_insert");
		mood_repository.create(event).await
	}?;

	Ok(Json(result))
}

#[axum::debug_handler]
#[instrument(name = "get_all_mood_events", skip(state), fields(otel.kind = "server"))]
pub async fn get_all_mood_events(State(state): State<AppState>) -> Result<Json<Vec<MoodEvent>>, DedupError> {
	let _timer = OperationTimer::new("get_all_mood_events", "total");

	let mood_repository = Arc::new(MoodEventRepository::new(state.core.shared_db));
	let cache_key = "get_all_mood_events".to_string();

	let (events, was_cached) = state
		.realtime
		.dedup_cache
		.get_or_fetch(&cache_key, || async {
			let _db_timer = OperationTimer::new("get_all_mood_events", "database_fetch");
			mood_repository.get_all().await.map_err(DedupError::from)
		})
		.await?;

	record_cache_hit("get_all_mood_events", was_cached);

	Ok(Json(events))
}

#[axum::debug_handler]
#[instrument(name = "get_mood_event_by_id", skip(state), fields(id = %id, otel.kind = "server"))]
pub async fn get_mood_event_by_id(State(state): State<AppState>, Path(id): Path<i64>) -> Result<Json<MoodEvent>, DedupError> {
	let _timer = OperationTimer::new("get_mood_event_by_id", "total");

	let mood_repository = Arc::new(MoodEventRepository::new(state.core.shared_db));
	let cache_key = format!("mood_event_{}", id);

	let (event_opt, was_cached) = state
		.realtime
		.dedup_cache
		.get_or_fetch(&cache_key, || async {
			let _db_timer = OperationTimer::new("get_mood_event_by_id", "database_fetch");
			mood_repository.get_by_id(id).await.map_err(DedupError::from)
		})
		.await?;

	record_cache_hit("get_mood_event_by_id", was_cached);

	match event_opt {
		Some(event) => Ok(Json(event)),
		None => Err(MoodEventError::NotFound.into()),
	}
}

#[axum::debug_handler]
#[instrument(name = "update_mood_event", skip(state), fields(id = %id, otel.kind = "server"))]
pub async fn update_mood_event(State(state): State<AppState>, Path(id): Path<i64>, Json(update): Json<UpdateMoodEvent>) -> Result<Json<MoodEvent>, DedupError> {
	let _timer = OperationTimer::new("update_mood_event", "total");

	let mood_repository = Arc::new(MoodEventRepository::new(state.core.shared_db));

	let result_opt = {
		let _db_timer = OperationTimer::new("update_mood_event", "database_update");
		mood_repository.update(id, update).await
	}?;

	let result = match result_opt {
		Some(event) => event,
		None => return Err(MoodEventError::NotFound.into()),
	};

	// Invalidate cache
	{
		let _cache_timer = OperationTimer::new("update_mood_event", "cache_invalidation");
		let cache_key = format!("mood_event_{}", id);
		state.realtime.dedup_cache.delete(&cache_key).await?;
		state.realtime.dedup_cache.delete("get_all_mood_events").await?;
		record_cache_invalidation("update_mood_event", 2);
	}

	Ok(Json(result))
}

#[axum::debug_handler]
#[instrument(name = "delete_mood_event", skip(state), fields(id = %id, otel.kind = "server"))]
pub async fn delete_mood_event(State(state): State<AppState>, Path(id): Path<i64>) -> Result<StatusCode, DedupError> {
	let _timer = OperationTimer::new("delete_mood_event", "total");

	let mood_repository = Arc::new(MoodEventRepository::new(state.core.shared_db));

	let deleted = {
		let _db_timer = OperationTimer::new("delete_mood_event", "database_delete");
		mood_repository.delete(id).await
	}?;

	if deleted {
		// Invalidate cache
		{
			let _cache_timer = OperationTimer::new("delete_mood_event", "cache_invalidation");
			let cache_key = format!("mood_event_{}", id);
			state.realtime.dedup_cache.delete(&cache_key).await?;
			state.realtime.dedup_cache.delete("get_all_mood_events").await?;
			record_cache_invalidation("delete_mood_event", 2);
		}
		Ok(StatusCode::NO_CONTENT)
	} else {
		Err(MoodEventError::NotFound.into())
	}
}

// Batch operation handlers
#[axum::debug_handler]
#[instrument(name = "batch_create_mood_events", skip(state), fields(event_count = %request.events.len(), otel.kind = "server"))]
pub async fn batch_create_mood_events(State(state): State<AppState>, Json(request): Json<BatchCreateRequest>) -> Result<Json<Vec<MoodEvent>>, DedupError> {
	let _timer = OperationTimer::new("batch_create_mood_events", "total");

	if request.events.is_empty() {
		return Err(MoodEventError::ValidationError("Events list cannot be empty".to_string()).into());
	}

	let mood_repository = Arc::new(MoodEventRepository::new(state.core.shared_db));

	let result = {
		let _db_timer = OperationTimer::new("batch_create_mood_events", "database_batch_insert");
		mood_repository.batch_create(request.events).await
	}?;

	// Invalidate relevant caches
	{
		let _cache_timer = OperationTimer::new("batch_create_mood_events", "cache_invalidation");
		state.realtime.dedup_cache.delete("get_all_mood_events").await?;
		record_cache_invalidation("batch_create_mood_events", 1);
	}

	Ok(Json(result))
}

#[axum::debug_handler]
#[instrument(name = "batch_update_mood_events", skip(state), fields(update_count = %request.updates.len(), otel.kind = "server"))]
pub async fn batch_update_mood_events(State(state): State<AppState>, Json(request): Json<BatchUpdateRequest>) -> Result<Json<Vec<MoodEvent>>, DedupError> {
	let _timer = OperationTimer::new("batch_update_mood_events", "total");

	if request.updates.is_empty() {
		return Err(MoodEventError::ValidationError("Updates map cannot be empty".to_string()).into());
	}

	let mood_repository = Arc::new(MoodEventRepository::new(state.core.shared_db));

	let result = {
		let _db_timer = OperationTimer::new("batch_update_mood_events", "database_batch_update");
		mood_repository.batch_update(request.updates.clone()).await
	}?;

	// Invalidate relevant caches
	{
		let _cache_timer = OperationTimer::new("batch_update_mood_events", "cache_invalidation");
		let keys_count = request.updates.len() + 1; // individual keys + get_all

		for id in request.updates.keys() {
			let cache_key = format!("mood_event_{}", id);
			state.realtime.dedup_cache.delete(&cache_key).await?;
		}
		state.realtime.dedup_cache.delete("get_all_mood_events").await?;

		record_cache_invalidation("batch_update_mood_events", keys_count);
	}

	Ok(Json(result))
}

#[axum::debug_handler]
#[instrument(name = "batch_delete_mood_events", skip(state), fields(delete_count = %request.ids.len(), otel.kind = "server"))]
pub async fn batch_delete_mood_events(State(state): State<AppState>, Json(request): Json<BatchDeleteRequest>) -> Result<Json<BatchDeleteResponse>, DedupError> {
	let _timer = OperationTimer::new("batch_delete_mood_events", "total");

	if request.ids.is_empty() {
		return Err(MoodEventError::ValidationError("IDs list cannot be empty".to_string()).into());
	}

	let mood_repository = Arc::new(MoodEventRepository::new(state.core.shared_db));

	let deleted_count = {
		let _db_timer = OperationTimer::new("batch_delete_mood_events", "database_batch_delete");
		mood_repository.batch_delete(request.ids.clone()).await
	}?;

	// Invalidate relevant caches
	{
		let _cache_timer = OperationTimer::new("batch_delete_mood_events", "cache_invalidation");
		let keys_count = request.ids.len() + 1; // individual keys + get_all

		for id in &request.ids {
			let cache_key = format!("mood_event_{}", id);
			state.realtime.dedup_cache.delete(&cache_key).await?;
		}
		state.realtime.dedup_cache.delete("get_all_mood_events").await?;

		record_cache_invalidation("batch_delete_mood_events", keys_count);
	}

	Ok(Json(BatchDeleteResponse { deleted_count }))
}

// Query handlers
#[axum::debug_handler]
#[instrument(name = "get_mood_events_by_week", skip(state), fields(week = %week, otel.kind = "server"))]
pub async fn get_mood_events_by_week(State(state): State<AppState>, Path(week): Path<i64>) -> Result<Json<Vec<MoodEvent>>, DedupError> {
	let _timer = OperationTimer::new("get_mood_events_by_week", "total");

	let mood_repository = Arc::new(MoodEventRepository::new(state.core.shared_db));
	let cache_key = format!("mood_events_week_{}", week);

	let (events, was_cached) = state
		.realtime
		.dedup_cache
		.get_or_fetch(&cache_key, || async {
			let _db_timer = OperationTimer::new("get_mood_events_by_week", "database_fetch");
			mood_repository.get_by_week(week).await.map_err(DedupError::from)
		})
		.await?;

	record_cache_hit("get_mood_events_by_week", was_cached);

	Ok(Json(events))
}

#[axum::debug_handler]
#[instrument(name = "get_mood_events_by_team", skip(state), fields(team = %team, otel.kind = "server"))]
pub async fn get_mood_events_by_team(State(state): State<AppState>, Path(team): Path<String>) -> Result<Json<Vec<MoodEvent>>, DedupError> {
	let _timer = OperationTimer::new("get_mood_events_by_team", "total");

	let mood_repository = Arc::new(MoodEventRepository::new(state.core.shared_db));
	let cache_key = format!("mood_events_team_{}", team);

	let (events, was_cached) = state
		.realtime
		.dedup_cache
		.get_or_fetch(&cache_key, || async {
			let _db_timer = OperationTimer::new("get_mood_events_by_team", "database_fetch");
			mood_repository.get_by_team(&team).await.map_err(DedupError::from)
		})
		.await?;

	record_cache_hit("get_mood_events_by_team", was_cached);

	Ok(Json(events))
}

#[axum::debug_handler]
#[instrument(name = "get_mood_stats", skip(state), fields(otel.kind = "server"))]
pub async fn get_mood_stats(State(state): State<AppState>) -> Result<Json<MoodStats>, DedupError> {
	let _timer = OperationTimer::new("get_mood_stats", "total");

	let mood_repository = Arc::new(MoodEventRepository::new(state.core.shared_db));
	let cache_key = "mood_stats".to_string();

	let (stats, was_cached) = state
		.realtime
		.dedup_cache
		.get_or_fetch(&cache_key, || async {
			let _db_timer = OperationTimer::new("get_mood_stats", "database_fetch");
			mood_repository.get_mood_stats().await.map_err(DedupError::from)
		})
		.await?;

	record_cache_hit("get_mood_stats", was_cached);

	Ok(Json(stats))
}
