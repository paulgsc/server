//! `/api/v1/sessions` — the surface the browser repositories need.
//!
//! The endpoint list was not open to invention. It is fixed by the methods
//! `SessionsRepository` already exposes in
//! `apps/www/src/lib/tenant/sessions-repository.ts` (`paulgsc/some-ui`), and
//! matching them one-to-one is what makes the client change a swap rather than
//! a rewrite:
//!
//! ```text
//! list()                    → GET    /sessions
//! get(id)                   → GET    /sessions/:id
//! create(input)             → POST   /sessions
//! update(id, patch)         → PATCH  /sessions/:id
//! remove(id)                → DELETE /sessions/:id
//! removeMany(ids)           → DELETE /sessions          { ids }
//! updateStatusMany(ids, s)  → PATCH  /sessions/status   { ids, status }
//! duplicate(id)             → POST   /sessions/:id/duplicate
//! ```
//!
//! Two behaviours were client-side and move with the data, or they diverge:
//! **id generation** (`generateId("session")` produced `session-<uuid>`) and
//! **`totalDurationOf(scenes)`**. Both are the server's now.
//!
//! Every mutating call returns the full record, because the client's
//! repositories already do and its react-query cache depends on it.

use crate::subject::SubjectId;
use crate::{AppState, FileHostError};
use axum::{
	extract::{Path, State},
	Json,
};
use chrono::Utc;
use serde::Deserialize;
use session_repo::{total_duration_of, CreateSession, SessionRecord, SessionRepository, SessionStatus, UpdateSession};
use tracing::instrument;

#[derive(Debug, Deserialize)]
pub struct RemoveManyRequest {
	pub ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct StatusManyRequest {
	pub ids: Vec<String>,
	pub status: SessionStatus,
}

/// `session-<uuid>`, mirroring the client's `generateId("session")`.
fn new_id() -> String {
	format!("session-{}", uuid::Uuid::new_v4())
}

fn repo(state: &AppState) -> SessionRepository {
	SessionRepository::new(state.core.shared_db.clone())
}

fn to_http(err: &session_repo::repository::SessionRepoError) -> FileHostError {
	FileHostError::OperationError(err.to_string())
}

/// `GET /sessions`
#[axum::debug_handler]
#[instrument(name = "list_sessions", skip_all, fields(otel.kind = "server"))]
pub async fn list_sessions(State(state): State<AppState>, subject: SubjectId) -> Result<Json<Vec<SessionRecord>>, FileHostError> {
	repo(&state).list(subject.as_str()).await.map(Json).map_err(|err| to_http(&err))
}

/// `GET /sessions/:id`
///
/// A missing id is a 404. The client throws `SessionNotFoundError` on it, and
/// a 500 would have it reporting an outage instead.
#[axum::debug_handler]
#[instrument(name = "get_session", skip_all, fields(otel.kind = "server"))]
pub async fn get_session(State(state): State<AppState>, subject: SubjectId, Path(id): Path<String>) -> Result<Json<SessionRecord>, FileHostError> {
	repo(&state)
		.get(subject.as_str(), &id)
		.await
		.map_err(|err| to_http(&err))?
		.map(Json)
		.ok_or(FileHostError::NotFound)
}

/// `POST /sessions`
#[axum::debug_handler]
#[instrument(name = "create_session", skip_all, fields(otel.kind = "server"))]
pub async fn create_session(State(state): State<AppState>, subject: SubjectId, Json(input): Json<CreateSession>) -> Result<Json<SessionRecord>, FileHostError> {
	let now = Utc::now().to_rfc3339();

	let record = SessionRecord {
		id: new_id(),
		name: input.name,
		// Everything starts as a draft. The client's `create` did the same, and
		// a session that could be born `scheduled` would be one the nudge could
		// offer before anyone had finished composing it.
		status: SessionStatus::Draft,
		// Computed here rather than trusted from the body: a client that
		// forgets sends a zero, and the nudge then offers a "~1 min" session.
		total_duration_ms: total_duration_of(&input.scenes),
		activities: input.activities,
		scenes: input.scenes,
		layout_mode: input.layout_mode,
		layout: input.layout,
		created_at: now.clone(),
		updated_at: now,
		started_at: None,
		completed_at: None,
		final_elapsed_ms: None,
	};

	repo(&state).upsert(subject.as_str(), &record).await.map_err(|err| to_http(&err))?;
	Ok(Json(record))
}

/// `PATCH /sessions/:id`
#[axum::debug_handler]
#[instrument(name = "update_session", skip_all, fields(otel.kind = "server"))]
pub async fn update_session(
	State(state): State<AppState>,
	subject: SubjectId,
	Path(id): Path<String>,
	Json(patch): Json<UpdateSession>,
) -> Result<Json<SessionRecord>, FileHostError> {
	let sessions = repo(&state);
	let mut record = sessions.get(subject.as_str(), &id).await.map_err(|err| to_http(&err))?.ok_or(FileHostError::NotFound)?;

	// A patch is `Partial<...>`: absent means "leave it". The one exception is
	// `layout`, where absent and explicit-null differ — see `deserialize_explicit_null`.
	if let Some(name) = patch.name {
		record.name = name;
	}
	if let Some(status) = patch.status {
		record.status = status;
	}
	if let Some(activities) = patch.activities {
		record.activities = activities;
	}
	if let Some(scenes) = patch.scenes {
		// Recomputed rather than carried, for the same reason `create` computes it.
		record.total_duration_ms = total_duration_of(&scenes);
		record.scenes = scenes;
	}
	if let Some(layout_mode) = patch.layout_mode {
		record.layout_mode = layout_mode;
	}
	if patch.layout.is_some() {
		record.layout = patch.layout;
	}
	// An explicit `totalDurationMs` wins over the derived one, because a client
	// editing scenes and duration in one patch means what it said.
	if let Some(total) = patch.total_duration_ms {
		record.total_duration_ms = total;
	}
	if let Some(started_at) = patch.started_at {
		record.started_at = Some(started_at);
	}
	if let Some(completed_at) = patch.completed_at {
		record.completed_at = Some(completed_at);
	}
	if let Some(elapsed) = patch.final_elapsed_ms {
		record.final_elapsed_ms = Some(elapsed);
	}

	record.updated_at = Utc::now().to_rfc3339();
	sessions.upsert(subject.as_str(), &record).await.map_err(|err| to_http(&err))?;
	Ok(Json(record))
}

/// `DELETE /sessions/:id`
#[axum::debug_handler]
#[instrument(name = "delete_session", skip_all, fields(otel.kind = "server"))]
pub async fn delete_session(State(state): State<AppState>, subject: SubjectId, Path(id): Path<String>) -> Result<Json<serde_json::Value>, FileHostError> {
	let removed = repo(&state).delete(subject.as_str(), &id).await.map_err(|err| to_http(&err))?;
	Ok(Json(serde_json::json!({ "removed": removed })))
}

/// `DELETE /sessions` with `{ ids }`
#[axum::debug_handler]
#[instrument(name = "delete_sessions", skip_all, fields(otel.kind = "server"))]
pub async fn delete_sessions(State(state): State<AppState>, subject: SubjectId, Json(request): Json<RemoveManyRequest>) -> Result<Json<serde_json::Value>, FileHostError> {
	let deleted = repo(&state).delete_many(subject.as_str(), &request.ids).await.map_err(|err| to_http(&err))?;
	Ok(Json(serde_json::json!({ "deletedCount": deleted })))
}

/// `PATCH /sessions/status` with `{ ids, status }`
#[axum::debug_handler]
#[instrument(name = "set_session_status", skip_all, fields(otel.kind = "server"))]
pub async fn set_status(State(state): State<AppState>, subject: SubjectId, Json(request): Json<StatusManyRequest>) -> Result<Json<Vec<SessionRecord>>, FileHostError> {
	repo(&state)
		.set_status_many(subject.as_str(), &request.ids, request.status, &Utc::now().to_rfc3339())
		.await
		.map(Json)
		.map_err(|err| to_http(&err))
}

/// `POST /sessions/:id/duplicate`
///
/// The reset semantics are the client's and are preserved deliberately: the
/// copy is a `draft` with `startedAt`/`completedAt` cleared and `(copy)`
/// appended. Carrying the timestamps over would make a fresh copy read as
/// "studied today" and silence the nudge for a day nobody studied.
#[axum::debug_handler]
#[instrument(name = "duplicate_session", skip_all, fields(otel.kind = "server"))]
pub async fn duplicate_session(State(state): State<AppState>, subject: SubjectId, Path(id): Path<String>) -> Result<Json<SessionRecord>, FileHostError> {
	let sessions = repo(&state);
	let source = sessions.get(subject.as_str(), &id).await.map_err(|err| to_http(&err))?.ok_or(FileHostError::NotFound)?;
	let now = Utc::now().to_rfc3339();

	let copy = SessionRecord {
		id: new_id(),
		name: format!("{} (copy)", source.name),
		status: SessionStatus::Draft,
		started_at: None,
		completed_at: None,
		created_at: now.clone(),
		updated_at: now,
		..source
	};

	sessions.upsert(subject.as_str(), &copy).await.map_err(|err| to_http(&err))?;
	Ok(Json(copy))
}

#[cfg(test)]
mod tests {
	use super::new_id;

	#[test]
	fn ids_match_the_shape_the_client_minted() {
		let id = new_id();
		assert!(id.starts_with("session-"), "got {id}");
		assert!(uuid::Uuid::parse_str(id.trim_start_matches("session-")).is_ok());
	}
}
