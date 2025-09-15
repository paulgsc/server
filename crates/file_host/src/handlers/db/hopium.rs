use crate::{metrics::http::OPERATION_DURATION, timed_operation, AppState, DedupError};
use axum::{
	extract::{Path, State},
	http::StatusCode,
	Json,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tracing::instrument;

use mood_event::core::{CreateMoodEvent, MoodEvent, MoodEventRepository, MoodStats, UpdateMoodEvent};

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
#[instrument(name = "create_mood_event", skip(state))]
pub async fn create_mood_event(State(state): State<AppState>, Json(event): Json<CreateMoodEvent>) -> Result<Json<MoodEvent>, DedupError> {
	let mood_repository = Arc::new(MoodEventRepository::new(state.shared_db));
	let result = timed_operation!("create_mood_event", "database_insert", false, { mood_repository.create(event).await })?;

	Ok(Json(result))
}

#[axum::debug_handler]
#[instrument(name = "get_all_mood_events", skip(state))]
pub async fn get_all_mood_events(State(state): State<AppState>) -> Result<Json<Vec<MoodEvent>>, DedupError> {
	let mood_repository = Arc::new(MoodEventRepository::new(state.shared_db));
	let cache_key = "get_all_mood_events".to_string();
	let (events, _) = state
		.dedup_cache
		.get_or_fetch(&cache_key, || async {
			timed_operation!("get_all_mood_events", "database_fetch", false, {
				mood_repository.get_all().await.map_err(DedupError::from)
			})
		})
		.await?;

	Ok(Json(events))
}

#[axum::debug_handler]
#[instrument(name = "get_mood_event_by_id", skip(state))]
pub async fn get_mood_event_by_id(State(state): State<AppState>, Path(id): Path<i64>) -> Result<Json<MoodEvent>, DedupError> {
	let mood_repository = Arc::new(MoodEventRepository::new(state.shared_db));
	let cache_key = format!("mood_event_{}", id);
	let (event_opt, _) = state
		.dedup_cache
		.get_or_fetch(&cache_key, || async {
			timed_operation!("get_mood_event_by_id", "database_fetch", false, {
				mood_repository.get_by_id(id).await.map_err(DedupError::from)
			})
		})
		.await?;

	match event_opt {
		Some(event) => Ok(Json(event)),
		None => Err(MoodEventError::NotFound.into()),
	}
}

#[axum::debug_handler]
#[instrument(name = "update_mood_event", skip(state))]
pub async fn update_mood_event(State(state): State<AppState>, Path(id): Path<i64>, Json(update): Json<UpdateMoodEvent>) -> Result<Json<MoodEvent>, DedupError> {
	let mood_repository = Arc::new(MoodEventRepository::new(state.shared_db));
	let result_opt = timed_operation!("update_mood_event", "database_update", false, { mood_repository.update(id, update).await })?;

	let result = match result_opt {
		Some(event) => event,
		None => return Err(MoodEventError::NotFound.into()),
	};

	// Invalidate cache
	let cache_key = format!("mood_event_{}", id);
	state.dedup_cache.delete(&cache_key).await?;
	state.dedup_cache.delete("get_all_mood_events").await?;

	Ok(Json(result))
}

#[axum::debug_handler]
#[instrument(name = "delete_mood_event", skip(state))]
pub async fn delete_mood_event(State(state): State<AppState>, Path(id): Path<i64>) -> Result<StatusCode, DedupError> {
	let mood_repository = Arc::new(MoodEventRepository::new(state.shared_db));
	let deleted = timed_operation!("delete_mood_event", "database_delete", false, { mood_repository.delete(id).await })?;

	if deleted {
		// Invalidate cache
		let cache_key = format!("mood_event_{}", id);
		state.dedup_cache.delete(&cache_key).await?;
		state.dedup_cache.delete("get_all_mood_events").await?;
		Ok(StatusCode::NO_CONTENT)
	} else {
		Err(MoodEventError::NotFound.into())
	}
}

// Batch operation handlers
#[axum::debug_handler]
#[instrument(name = "batch_create_mood_events", skip(state))]
pub async fn batch_create_mood_events(State(state): State<AppState>, Json(request): Json<BatchCreateRequest>) -> Result<Json<Vec<MoodEvent>>, DedupError> {
	if request.events.is_empty() {
		return Err(MoodEventError::ValidationError("Events list cannot be empty".to_string()).into());
	}

	let mood_repository = Arc::new(MoodEventRepository::new(state.shared_db));
	let result = timed_operation!("batch_create_mood_events", "database_batch_insert", false, {
		mood_repository.batch_create(request.events).await
	})?;

	// Invalidate relevant caches
	state.dedup_cache.delete("get_all_mood_events").await?;

	Ok(Json(result))
}

#[axum::debug_handler]
#[instrument(name = "batch_update_mood_events", skip(state))]
pub async fn batch_update_mood_events(State(state): State<AppState>, Json(request): Json<BatchUpdateRequest>) -> Result<Json<Vec<MoodEvent>>, DedupError> {
	if request.updates.is_empty() {
		return Err(MoodEventError::ValidationError("Updates map cannot be empty".to_string()).into());
	}

	let mood_repository = Arc::new(MoodEventRepository::new(state.shared_db));
	let result = timed_operation!("batch_update_mood_events", "database_batch_update", false, {
		mood_repository.batch_update(request.updates.clone()).await
	})?;

	// Invalidate relevant caches
	for id in request.updates.keys() {
		let cache_key = format!("mood_event_{}", id);
		state.dedup_cache.delete(&cache_key).await?;
	}
	state.dedup_cache.delete("get_all_mood_events").await?;

	Ok(Json(result))
}

#[axum::debug_handler]
#[instrument(name = "batch_delete_mood_events", skip(state))]
pub async fn batch_delete_mood_events(State(state): State<AppState>, Json(request): Json<BatchDeleteRequest>) -> Result<Json<BatchDeleteResponse>, DedupError> {
	if request.ids.is_empty() {
		return Err(MoodEventError::ValidationError("IDs list cannot be empty".to_string()).into());
	}

	let mood_repository = Arc::new(MoodEventRepository::new(state.shared_db));
	let deleted_count = timed_operation!("batch_delete_mood_events", "database_batch_delete", false, {
		mood_repository.batch_delete(request.ids.clone()).await
	})?;

	// Invalidate relevant caches
	for id in &request.ids {
		let cache_key = format!("mood_event_{}", id);
		state.dedup_cache.delete(&cache_key).await?;
	}
	state.dedup_cache.delete("get_all_mood_events").await?;

	Ok(Json(BatchDeleteResponse { deleted_count }))
}

// Query handlers
#[axum::debug_handler]
#[instrument(name = "get_mood_events_by_week", skip(state))]
pub async fn get_mood_events_by_week(State(state): State<AppState>, Path(week): Path<i64>) -> Result<Json<Vec<MoodEvent>>, DedupError> {
	let mood_repository = Arc::new(MoodEventRepository::new(state.shared_db));
	let cache_key = format!("mood_events_week_{}", week);
	let (events, _) = state
		.dedup_cache
		.get_or_fetch(&cache_key, || async {
			timed_operation!("get_mood_events_by_week", "database_fetch", false, {
				mood_repository.get_by_week(week).await.map_err(DedupError::from)
			})
		})
		.await?;

	Ok(Json(events))
}

#[axum::debug_handler]
#[instrument(name = "get_mood_events_by_team", skip(state))]
pub async fn get_mood_events_by_team(State(state): State<AppState>, Path(team): Path<String>) -> Result<Json<Vec<MoodEvent>>, DedupError> {
	let mood_repository = Arc::new(MoodEventRepository::new(state.shared_db));
	let cache_key = format!("mood_events_team_{}", team);
	let (events, _) = state
		.dedup_cache
		.get_or_fetch(&cache_key, || async {
			timed_operation!("get_mood_events_by_team", "database_fetch", false, {
				mood_repository.get_by_team(&team).await.map_err(DedupError::from)
			})
		})
		.await?;

	Ok(Json(events))
}

#[axum::debug_handler]
#[instrument(name = "get_mood_stats", skip(state))]
pub async fn get_mood_stats(State(state): State<AppState>) -> Result<Json<MoodStats>, DedupError> {
	let mood_repository = Arc::new(MoodEventRepository::new(state.shared_db));
	let cache_key = "mood_stats".to_string();

	let (stats, _) = state
		.dedup_cache
		.get_or_fetch(&cache_key, || async {
			timed_operation!("get_mood_stats", "database_fetch", false, {
				mood_repository.get_mood_stats().await.map_err(DedupError::from)
			})
		})
		.await?;

	Ok(Json(stats))
}
