//! `/api/v1/sessions` — the surface the browser repositories need.
//!
//! The endpoint list was not open to invention. It is fixed by the methods
//! `SessionsRepository` already exposes in
//! `apps/www/src/lib/tenant/sessions-repository/index.ts` (`paulgsc/some-ui`), and
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
//! **`totalDurationOf(scenes)`**. Both are the server's now, but for one
//! case: a create may name its id, and the same create sent again then
//! answers with the session the first made (`create_session`).
//!
//! Every mutating call returns the full record, because the client's
//! repositories already do and its react-query cache depends on it.

use crate::nudge::clock::parse_timestamp;
use crate::subject::SubjectId;
use crate::{AppState, FileHostError};
use axum::{
	extract::{Path, State},
	Json,
};
use chrono::{DateTime, TimeDelta, Utc};
use serde::Deserialize;
use session_repo::{is_session_id, new_session_id, total_duration_of, CreateSession, Created, SessionOrigin, SessionRecord, SessionRepository, SessionStatus, UpdateSession};
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

fn repo(state: &AppState) -> SessionRepository {
	SessionRepository::new(state.core.shared_db.clone())
}

fn to_http(err: &session_repo::repository::SessionRepoError) -> FileHostError {
	FileHostError::OperationError(err.to_string())
}

/// How far past home's clock a sent `createdAt` may be: five minutes. A
/// phone sets its clock from the network, so a session it made a moment
/// before copying it home can read a little ahead of home's clock, but not
/// by minutes. A stamp further ahead is a clock set wrong, and a session
/// dated into the future would head the list (`ORDER BY created_at DESC`)
/// until that day came, so it is refused rather than kept or moved.
const CREATED_AT_AHEAD: TimeDelta = TimeDelta::minutes(5);

/// A sent `createdAt` as home stores it, for a copy of a session made
/// elsewhere (the phone's, or this browser's), which keeps when it was made
/// there: an RFC 3339 time no more than [`CREATED_AT_AHEAD`] past `now`,
/// in UTC and in the form home writes its own stamps (`to_rfc3339`), so
/// `ORDER BY created_at` still compares like with like. How far back it
/// goes is the client's to say: a copy of a session made long ago is dated
/// long ago. Absent, a create is dated now and a patch leaves the date;
/// `upsert` never moves it, so a patch's goes through
/// `SessionRepository::set_created_at`.
///
/// # Errors
/// What is wrong with it, for the `422` naming `createdAt`.
fn created_at_of(sent: &str, now: DateTime<Utc>) -> Result<String, &'static str> {
	let at = parse_timestamp(sent).ok_or("is an RFC 3339 time")?;
	if at > now + CREATED_AT_AHEAD {
		return Err("is no more than five minutes past home's clock");
	}
	Ok(at.to_rfc3339())
}

/// [`created_at_of`] for a request: absent stays absent, and a refusal is
/// the `422` that names the field.
fn sent_created_at(sent: Option<&str>, now: DateTime<Utc>) -> Result<Option<String>, FileHostError> {
	sent
		.map(|sent| created_at_of(sent, now))
		.transpose()
		.map_err(|problem| FileHostError::unprocessable_entity([("createdAt", problem)]))
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
///
/// Once per id (`SessionRepository::insert_if_absent`): a create under an
/// id this subject already holds answers with that session as it is now and
/// changes nothing, so a client that names its own id can send a create
/// whose answer was lost again. An id another subject holds is a `409`; a
/// named one that is not [`is_session_id`] a `422`; without one, the server
/// mints it. A `createdAt` is [`created_at_of`]'s.
#[axum::debug_handler]
#[instrument(name = "create_session", skip_all, fields(otel.kind = "server"))]
pub async fn create_session(State(state): State<AppState>, subject: SubjectId, Json(input): Json<CreateSession>) -> Result<Json<SessionRecord>, FileHostError> {
	let clock = Utc::now();
	let now = clock.to_rfc3339();
	let created_at = sent_created_at(input.created_at.as_deref(), clock)?.unwrap_or_else(|| now.clone());
	let id = match input.id {
		Some(id) if is_session_id(&id) => id,
		Some(_) => return Err(FileHostError::unprocessable_entity([("id", "is session- and 16 to 56 letters, digits, - and _")])),
		None => new_session_id(),
	};

	let record = SessionRecord {
		id,
		name: input.name,
		// Everything starts as a draft. The client's `create` did the same, and
		// a session that could be born `scheduled` would be one the nudge could
		// offer before anyone had finished composing it.
		status: SessionStatus::Draft,
		// A session created through this endpoint is always someone composing
		// it themselves — the waker's own path (`nudge::waker::
		// materialize_provisioned_session`) never calls `create_session`, it
		// writes directly through `SessionRepository::provision_if_absent`.
		origin: SessionOrigin::User,
		// Computed here rather than trusted from the body: a client that
		// forgets sends a zero, and the nudge then offers a "~1 min" session.
		total_duration_ms: total_duration_of(&input.scenes),
		activities: input.activities,
		scenes: input.scenes,
		layout_mode: input.layout_mode,
		layout: input.layout,
		created_at,
		updated_at: now,
		started_at: None,
		completed_at: None,
		final_elapsed_ms: None,
	};

	match repo(&state).insert_if_absent(subject.as_str(), &record).await.map_err(|err| to_http(&err))? {
		Created::Inserted => Ok(Json(record)),
		Created::Held(held) => Ok(Json(*held)),
		Created::Taken => Err(FileHostError::Conflict("another session holds this id")),
	}
}

/// `PATCH /sessions/:id`
///
/// A `createdAt` is [`created_at_of`]'s.
#[axum::debug_handler]
#[instrument(name = "update_session", skip_all, fields(otel.kind = "server"))]
pub async fn update_session(
	State(state): State<AppState>,
	subject: SubjectId,
	Path(id): Path<String>,
	Json(patch): Json<UpdateSession>,
) -> Result<Json<SessionRecord>, FileHostError> {
	let sessions = repo(&state);
	let clock = Utc::now();
	let created_at = sent_created_at(patch.created_at.as_deref(), clock)?;
	let mut record = sessions.get(subject.as_str(), &id).await.map_err(|err| to_http(&err))?.ok_or(FileHostError::NotFound)?;

	// A patch is `Partial<...>`: absent means "leave it". The one exception is
	// `layout`, where absent and explicit-null differ — see `deserialize_explicit_null`.
	if let Some(name) = patch.name {
		record.name = name;
	}
	if let Some(status) = patch.status {
		record.status = status;
	}
	// The one-directional `system → user` rule is not trusted from this
	// field alone — `SessionRepository::upsert` enforces it regardless of
	// what is set here, so a `system` value sent against an already-`user`
	// row is silently ignored at the write, not here.
	if let Some(origin) = patch.origin {
		record.origin = origin;
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

	record.updated_at = clock.to_rfc3339();
	sessions.upsert(subject.as_str(), &record).await.map_err(|err| to_http(&err))?;
	if let Some(created_at) = created_at {
		sessions.set_created_at(subject.as_str(), &record.id, &created_at).await.map_err(|err| to_http(&err))?;
	}

	// `origin` is the one field `upsert` can silently refuse to apply — the
	// `system → user` one-way guard — so returning the in-memory `record`
	// directly could report a `system` a caller sent even though the row
	// stayed `user` in the database. Re-reading after the write is what
	// makes the response agree with what is actually persisted; a real
	// Codex finding on this PR (`paulgsc/server#334`) caught exactly this.
	sessions
		.get(subject.as_str(), &record.id)
		.await
		.map_err(|err| to_http(&err))?
		.map(Json)
		.ok_or(FileHostError::NotFound)
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
///
/// **`origin: user`, always** — a real, deliberate departure from the plain
/// `..source` spread every other field uses. Pressing "duplicate" is an
/// action nobody but a person can take (the waker's own path never calls
/// this handler), so a duplicate of a `system` proposal is not a second
/// proposal, it is something that person just did — the same class of
/// judgment PRO1 (`paulgsc/some-ui#1052`) makes for renaming, only stronger:
/// minting a whole new row is a clearer act of ownership than editing an
/// existing one.
#[axum::debug_handler]
#[instrument(name = "duplicate_session", skip_all, fields(otel.kind = "server"))]
pub async fn duplicate_session(State(state): State<AppState>, subject: SubjectId, Path(id): Path<String>) -> Result<Json<SessionRecord>, FileHostError> {
	let sessions = repo(&state);
	let source = sessions.get(subject.as_str(), &id).await.map_err(|err| to_http(&err))?.ok_or(FileHostError::NotFound)?;
	let now = Utc::now().to_rfc3339();

	let copy = SessionRecord {
		id: new_session_id(),
		name: format!("{} (copy)", source.name),
		status: SessionStatus::Draft,
		origin: SessionOrigin::User,
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
	use super::{created_at_of, CREATED_AT_AHEAD};
	use chrono::{DateTime, TimeDelta, Utc};

	fn at(text: &str) -> DateTime<Utc> {
		DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc)
	}

	const NOW: &str = "2026-11-01T12:00:00+00:00";

	/// A copy keeps when it was made, however long ago, in the form home
	/// writes its own stamps: the phone's `Z` and an offset both land in UTC.
	#[test]
	fn a_copy_keeps_when_it_was_made() {
		let now = at(NOW);
		assert_eq!(created_at_of("2026-10-14T09:00:00.000Z", now), Ok(String::from("2026-10-14T09:00:00+00:00")));
		assert_eq!(created_at_of("2026-10-14T09:00:00.250Z", now), Ok(String::from("2026-10-14T09:00:00.250+00:00")));
		assert_eq!(created_at_of("2026-10-14T18:00:00+09:00", now), Ok(String::from("2026-10-14T09:00:00+00:00")));
		assert_eq!(created_at_of("2019-01-01T00:00:00Z", now), Ok(String::from("2019-01-01T00:00:00+00:00")));
		assert_eq!(
			created_at_of(&now.to_rfc3339(), now),
			Ok(now.to_rfc3339()),
			"home's own stamp reads back as itself, so a list mixing both still sorts"
		);
	}

	/// A phone a little ahead of home is kept; one further ahead is refused,
	/// not moved, so the phone hears of it.
	#[test]
	fn a_stamp_ahead_of_home_is_kept_only_within_the_drift() {
		let now = at(NOW);
		let edge = now + CREATED_AT_AHEAD;
		assert_eq!(created_at_of(&edge.to_rfc3339(), now), Ok(edge.to_rfc3339()));
		let past = edge + TimeDelta::milliseconds(1);
		assert!(created_at_of(&past.to_rfc3339(), now).is_err());
		assert!(created_at_of("2027-01-01T00:00:00Z", now).is_err());
	}

	#[test]
	fn a_stamp_that_is_no_time_is_refused() {
		let now = at(NOW);
		for sent in ["2026-10-14", "2026-10-14T09:00:00"] {
			assert_eq!(created_at_of(sent, now), Err("is an RFC 3339 time"), "{sent}");
		}
	}
}
