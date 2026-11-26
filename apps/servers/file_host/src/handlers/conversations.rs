//! Handlers for `routes::conversations`: a conversation essay's record and
//! the previews it offers (HOST16-S, paulgsc/server#1243).
//!
//! The record is `conversation_repo`'s, an append-only log replayed before
//! each event is kept; the previews are files home's renderer made
//! (`conversations`). These validate and answer.
//!
//! **A session's for its own subject** (a paired phone's or a browser's, as
//! Studio writes the owner's own events): a request without a session is
//! `401`, and a write from an untrusted origin `403`. An id that is not a
//! `showId` is `404`.
//!
//! Nothing of a record reaches a log: each handler records only `otel.kind`.

use crate::conversations::{self, Folder, Previews, Unoffered};
use crate::subject::SubjectId;
use crate::FileHostError;
use axum::{
	body::{Body, Bytes},
	extract::{Path, Request, State},
	http::StatusCode,
	response::{IntoResponse, Response},
	Json,
};
use chrono::{SecondsFormat, Utc};
use conversation_repo::step::{self, is_id, Refusal};
use conversation_repo::{ConversationError, Kept, Prepared};
use serde::Serialize;
use serde_json::Value;
use sqlx::SqlitePool;
use tower::ServiceExt;
use tower_http::services::ServeFile;
use tracing::{instrument, warn};

fn id(conversation: &str) -> Result<(), FileHostError> {
	if is_id(conversation) {
		Ok(())
	} else {
		Err(FileHostError::NotFound)
	}
}

fn drive(err: &std::io::Error) -> FileHostError {
	// Its kind only: never a path.
	warn!(kind = ?err.kind(), "the drive refused a conversation's previews");
	FileHostError::OperationError("the drive refused a conversation's previews".to_owned())
}

fn stored(err: ConversationError) -> FileHostError {
	match err {
		ConversationError::Storage(err) => FileHostError::Sqlite(err),
		ConversationError::Refused { .. } | ConversationError::Unreplayable { .. } => FileHostError::OperationError(err.to_string()),
	}
}

/// `GET /conversations/:id`: the record as `some-ui.conversation/2` keeps
/// it, oldest event first.
///
/// # Errors
/// 401 without a session, 404 when home keeps no event of it, and 500 for a
/// storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "conversations_get", skip_all, fields(otel.kind = "server"))]
pub async fn get_record(subject: SubjectId, State(db): State<SqlitePool>, Path(conversation): Path<String>) -> Result<Json<Value>, FileHostError> {
	id(&conversation)?;
	let mut conn = db.acquire().await?;
	let record = conversation_repo::record(&mut conn, subject.as_str(), &conversation).await.map_err(stored)?;
	record.map(Json).ok_or(FileHostError::NotFound)
}

/// `GET /conversations/:id/previews`: the previews it offers, by id, each
/// with its variants, their videos and the record's approval of it.
///
/// # Errors
/// 401 without a session, 404 for an id that is none, and 500 for a storage
/// or drive failure, or a kept log that does not replay.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "conversations_previews", skip_all, fields(otel.kind = "server"))]
pub async fn get_previews(subject: SubjectId, State(db): State<SqlitePool>, Path(conversation): Path<String>) -> Result<Json<Previews>, FileHostError> {
	Ok(Json(previews_of(&db, subject.as_str(), &Folder::home(), &conversation).await?))
}

/// [`get_previews`] for `subject`, with its previews in `folder`.
async fn previews_of(db: &SqlitePool, subject: &str, folder: &Folder, conversation: &str) -> Result<Previews, FileHostError> {
	id(conversation)?;
	let mut conn = db.acquire().await?;
	let state = conversation_repo::state(&mut conn, subject, conversation).await.map_err(stored)?;
	drop(conn);
	conversations::previews(folder, subject, conversation, state.approvals()).await.map_err(|err| drive(&err))
}

/// `GET /conversations/:id/previews/:preview/:variant/video`: the variant's
/// video, streamed, with ranges.
///
/// # Errors
/// 401 without a session, and 404 for a name that is no id or a video home
/// does not hold.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "conversations_video", skip_all, fields(otel.kind = "server"))]
pub async fn get_video(subject: SubjectId, Path((conversation, preview, variant)): Path<(String, String, String)>, request: Request) -> Result<Response, FileHostError> {
	// The caller's own folder only: another subject's video is a 404.
	let file = Folder::home().video(subject.as_str(), &conversation, &preview, &variant).ok_or(FileHostError::NotFound)?;
	if !tokio::fs::try_exists(&file).await.map_err(|err| drive(&err))? {
		return Err(FileHostError::NotFound);
	}
	let served = match ServeFile::new(file).oneshot(request).await {
		Ok(served) => served,
		Err(never) => match never {},
	};
	Ok(served.map(Body::new).into_response())
}

/// A refused event: why, and the index it would have had.
#[derive(Debug, Serialize)]
pub struct Refused<R> {
	pub refused: R,
	pub at: usize,
}

fn refused<R: Serialize>(refusal: R, at: usize) -> Response {
	(StatusCode::CONFLICT, Json(Refused { refused: refusal, at })).into_response()
}

/// `POST /conversations/:id/events`: keeps one `some-ui.conversation/2`
/// event, the body, at the log's end.
///
/// It is kept once the record's step takes it after the kept log, in one
/// `BEGIN IMMEDIATE` transaction. An approval must also name a preview home
/// offers, exactly as home offers it ([`conversations::offered`]).
///
/// `201 { kept, events }` when kept; `200` with the same when the body is
/// the log's last event already (a retried send keeps nothing twice); `409
/// { refused, at }` when the step refuses it, with its `ConversationRefusal`,
/// or home offers no such preview (`preview-unknown`, `preview-offer`). A
/// refused event writes nothing.
///
/// # Errors
/// 401 without a session, 403 from an untrusted origin, 404 for an id that is
/// none, 422 naming each field that is not the
/// schema's, or a body over `conversation_repo::BODY_CEILING`, and 500 for a storage or drive failure, or a
/// kept log that does not replay.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "conversations_post_event", skip_all, fields(otel.kind = "server"))]
pub async fn post_event(subject: SubjectId, State(db): State<SqlitePool>, Path(conversation): Path<String>, body: Bytes) -> Result<Response, FileHostError> {
	post(&db, subject.as_str(), &Folder::home(), &conversation, &body).await
}

/// [`post_event`] for `subject`, with its previews in `folder`.
async fn post(db: &SqlitePool, subject: &str, folder: &Folder, conversation: &str, body: &[u8]) -> Result<Response, FileHostError> {
	id(conversation)?;
	if body.len() > conversation_repo::BODY_CEILING {
		return Err(FileHostError::unprocessable_entity([("body", "is over the size ceiling")]));
	}
	let value: Value = serde_json::from_slice(body).map_err(|_unreadable| FileHostError::unprocessable_entity([("body", "is not JSON")]))?;
	let event = step::parse(value).map_err(FileHostError::unprocessable_entity)?;
	// An approval's preview is read before the write lock is taken, and
	// confirmed inside it by one `stat`: no file is read while it is held.
	let offer = match event.approval() {
		Some(approval) => {
			let stamp = conversations::stamp(folder, subject, conversation, &approval.preview).await.map_err(|err| drive(&err))?;
			let offer = conversations::offered(folder, subject, conversation, approval).await.map_err(|err| drive(&err))?;
			Some((approval, stamp, offer))
		}
		None => None,
	};
	let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let at = match conversation_repo::prepare(&mut tx, subject, conversation, &event).await {
		Ok(Prepared::Unchanged(kept)) => return Ok((StatusCode::OK, Json(kept)).into_response()),
		Ok(Prepared::Next(at)) => at,
		Err(ConversationError::Refused { at, refusal }) => return Ok(refused::<Refusal>(refusal, at)),
		Err(err) => return Err(stored(err)),
	};
	if let Some((approval, stamp, offer)) = offer {
		let now_stamp = conversations::stamp(folder, subject, conversation, &approval.preview).await.map_err(|err| drive(&err))?;
		// The preview changed since it was read: read it again (rare).
		let offer = if now_stamp == stamp {
			offer
		} else {
			conversations::offered(folder, subject, conversation, approval).await.map_err(|err| drive(&err))?
		};
		if let Err(unoffered) = offer {
			return Ok(refused::<Unoffered>(unoffered, at));
		}
	}
	let kept: Kept = conversation_repo::insert(&mut tx, subject, conversation, &event, at, &now).await.map_err(stored)?;
	tx.commit().await?;
	Ok((StatusCode::CREATED, Json(kept)).into_response())
}

#[cfg(test)]
mod tests;
