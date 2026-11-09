//! Handlers for a week's record and the card-era log (RR14).
//!
//! These are `routes::rust_rounds`' validate-and-answer half
//! (paulgsc/some-ui#2689). What a week sent, a paste and a session must be, and the store's
//! rules (sent once, append-only, a session grows by revisions), are
//! `rust_round_repo`'s `sent` and `log`.
//!
//! **A week's record is read, never stored whole.** It is the week (the
//! routine's, in the mailbox), what the owner sent of it, the errors pasted
//! in it, and how often each ticket was opened, read from the phone's
//! engagement events in `app_event` (CK2, ENG4) rather than a counter of
//! its own. A ticket no take answered by the deadline is a silence, and the
//! record says so whether or not the phone ever sent anything: not answering
//! is an answer.
//!
//! **Everything is the caller's.** Every handler takes a [`Paired`] phone
//! and reads and writes only under its subject: another subject's week is
//! a `404`, and its Send, pastes and sessions are not there (docs/identity.md
//! invariant 28). The [`Paired`] extractor holds off account deletion while
//! a handler runs (invariant 9). Nothing of a body reaches a log line: the
//! spans skip every argument (invariant 6).

use crate::handlers::rust_rounds::{mailbox_failure, now, reads_as_named, tagged};
use crate::subject::Paired;
use crate::FileHostError;
use axum::{
	body::Bytes,
	extract::{Path, State},
	http::{HeaderMap, StatusCode},
	response::{IntoResponse, Response},
	Json,
};
use chrono::{DateTime, SecondsFormat, Utc};
use rust_round_repo::log::{LogChange, SessionEntry, SESSION_REWRITTEN};
use rust_round_repo::sent::{Answer, Paste, Sent, StoredPaste, PASTED_TWICE, SENT_TWICE};
use rust_round_repo::week::Week;
use rust_round_repo::{Change, RoundError};
use serde::Serialize;
use sqlx::{SqliteConnection, SqlitePool};
use tracing::instrument;

/// The pipeline and affordance the phone's ticket opens are engagement
/// events of (paulgsc/some-ui `packages/engagement`, the `rust-rounds`
/// pipeline): one `left` at the ticket's place, `ticket-<n>` by the week's
/// order, each time a ticket's card is swiped away or the screen goes.
const PIPELINE: &str = "rust-rounds";
const TICKET_AFFORDANCE: &str = "week-ticket";

/// The place a week's `n`th ticket (from 1) is left at.
fn ticket_place(n: usize) -> String {
	String::from("ticket-") + &n.to_string()
}

/// `PUT /rust-weeks/:id/sent`.
#[derive(Debug, Serialize)]
pub struct SentWritten {
	/// `added` or `unchanged`.
	pub change: &'static str,
	/// The Send as home keeps it.
	pub sent: Sent,
}

/// `PUT /rust-weeks/:id/pastes/:at`.
#[derive(Debug, Serialize)]
pub struct PasteWritten {
	/// `added` or `unchanged`.
	pub change: &'static str,
	/// The paste as home keeps it: `at`, `text` and `note`.
	pub paste: Paste,
}

/// `GET /rust-weeks/:id/record`: the week's record, silence included.
#[derive(Debug, Serialize)]
pub struct WeekRecord {
	pub week: String,
	/// When the week opened, as the week writes it.
	pub opens: String,
	/// The deadline, as the week writes it.
	pub due: String,
	/// Home's clock is past `due`: nothing sent after it counts.
	pub closed: bool,
	/// The owner's Send, or null when home holds none.
	pub sent: Option<SentAt>,
	/// Each ticket, in the week's order.
	pub tickets: Vec<TicketRecord>,
	/// The tickets no take answered, in the week's order: every one when
	/// nothing was sent.
	pub unanswered: Vec<String>,
	/// The errors pasted in the week, in the order they were made.
	pub pastes: Vec<StoredPaste>,
}

/// When a week was sent.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SentAt {
	/// When Send was pressed on the phone.
	pub at: String,
	/// When it reached home.
	pub received_at: String,
}

/// One ticket of a week's record.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TicketRecord {
	pub ticket: String,
	pub answered: bool,
	/// The take's voice-note id, when answered.
	pub take: Option<String>,
	/// When the take was recorded, when answered.
	pub recorded_at: Option<String>,
	/// How many times its card was opened between the week's opening and its
	/// deadline.
	pub opens: i64,
}

/// `PUT /rust-round-log/sessions/:id`.
#[derive(Debug, Serialize)]
pub struct SessionWritten {
	/// `added`, `unchanged` or `extended`.
	pub change: &'static str,
	pub id: String,
	/// The revision home now holds as the session's last.
	pub revision: i64,
}

/// `GET /rust-round-log/sessions`.
#[derive(Debug, Serialize)]
pub struct SessionList {
	pub sessions: Vec<SessionEntry>,
}

/// The week `id` under `subject`, with a window, or a `404`.
async fn held_week(conn: &mut SqliteConnection, subject: &str, id: &str) -> Result<Week, FileHostError> {
	if !rust_round_repo::week::is_week_id(id) {
		return Err(FileHostError::NotFound);
	}
	let held = mailbox_repo::rust_week(conn, subject, &rust_round_repo::week::mailbox_name(id))
		.await
		.map_err(mailbox_failure)?
		.ok_or(FileHostError::NotFound)?;
	reads_as_named(id, &held.body).ok_or(FileHostError::NotFound)
}

fn refused(err: RoundError, words: &'static str) -> FileHostError {
	match err {
		RoundError::Rewritten => FileHostError::Conflict(words),
		RoundError::Storage(err) => FileHostError::Sqlite(err),
	}
}

const fn change_word(change: Change) -> (StatusCode, &'static str) {
	match change {
		Change::Added => (StatusCode::CREATED, "added"),
		Change::Unchanged => (StatusCode::OK, "unchanged"),
	}
}

/// An instant as `app_event.at` is stored: UTC, to the millisecond.
fn stored_instant(at: &str) -> Option<String> {
	let at = DateTime::parse_from_rfc3339(at).ok()?;
	Some(at.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Millis, true))
}

/// `PUT /rust-weeks/:id/sent`, the body being the phone's `WeekSent`: kept
/// once, in a `BEGIN IMMEDIATE` transaction.
///
/// # Errors
/// 401 without a session, 403 for a browser's or from an untrusted origin,
/// 404 for a week the caller's subject does not hold, 422 naming each
/// refused field (`at` after the deadline among them), 409 for another Send
/// of a week home holds one of, 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "rust_week_sent_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_sent(paired: Paired, State(db): State<SqlitePool>, Path(id): Path<String>, body: Bytes) -> Result<Response, FileHostError> {
	let now = now();
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let week = held_week(&mut tx, paired.subject.as_str(), &id).await?;
	let sent = rust_round_repo::sent::read_sent(&body, &week, &now).map_err(FileHostError::unprocessable_entity)?;
	let change = rust_round_repo::sent::put_sent(&mut tx, paired.subject.as_str(), &sent, &now)
		.await
		.map_err(|err| refused(err, SENT_TWICE))?;
	tx.commit().await?;
	let (status, change) = change_word(change);
	Ok((status, Json(SentWritten { change, sent })).into_response())
}

/// `PUT /rust-weeks/:id/pastes/:at`, the body being the paste as the phone
/// keeps it, less its week.
///
/// # Errors
/// 401 without a session, 403 for a browser's or from an untrusted origin,
/// 404 for a week the caller's subject does not hold, 422 naming each
/// refused field, 409 for another paste at the same moment, 500 for a
/// storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "rust_week_paste_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_paste(paired: Paired, State(db): State<SqlitePool>, Path((id, at)): Path<(String, String)>, body: Bytes) -> Result<Response, FileHostError> {
	let now = now();
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let week = held_week(&mut tx, paired.subject.as_str(), &id).await?;
	let paste = rust_round_repo::sent::read_paste(&body, &at, &now).map_err(FileHostError::unprocessable_entity)?;
	let change = rust_round_repo::sent::put_paste(&mut tx, paired.subject.as_str(), &week.id, &paste, &now)
		.await
		.map_err(|err| refused(err, PASTED_TWICE))?;
	tx.commit().await?;
	let (status, change) = change_word(change);
	Ok((status, Json(PasteWritten { change, paste })).into_response())
}

/// `GET /rust-weeks/:id/record`: the week, what was sent of it, its pastes,
/// and each ticket's opens.
///
/// # Errors
/// 401 without a session, 403 for a browser's, 404 for a week the caller's
/// subject does not hold or whose deadline home cannot read, 500 for a
/// storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "rust_week_record", skip_all, fields(otel.kind = "server"))]
pub async fn get_record(paired: Paired, State(db): State<SqlitePool>, Path(id): Path<String>) -> Result<Json<WeekRecord>, FileHostError> {
	let subject = paired.subject.as_str();
	let mut conn = db.acquire().await?;
	let week = held_week(&mut conn, subject, &id).await?;
	let Some(window) = week.window.clone() else {
		tracing::error!(kind = "no_window", "a week home holds has no deadline it can read; no record");
		return Err(FileHostError::NotFound);
	};
	let (Some(from), Some(to)) = (stored_instant(&window.opens), stored_instant(&window.due)) else {
		return Err(FileHostError::NotFound);
	};
	let sent = rust_round_repo::sent::sent(&mut conn, subject, &week.id).await?;
	let pastes = rust_round_repo::sent::pastes(&mut conn, subject, &week.id).await?;
	let opens = app_event_repo::left_by_place(&mut conn, subject, PIPELINE, TICKET_AFFORDANCE, &from, &to)
		.await
		.map_err(|err| FileHostError::OperationError(err.to_string()))?;
	let answers: &[Answer] = sent.as_ref().map_or(&[], |held| held.sent.answers.as_slice());
	let mut tickets = Vec::with_capacity(week.programs.len());
	let mut unanswered = Vec::new();
	for (index, program) in week.programs.iter().enumerate() {
		let place = ticket_place(index + 1);
		let opened = opens.iter().find(|each| each.place == place).map_or(0, |each| each.left);
		let answer = answers.iter().find(|answer| answer.ticket == program.ticket);
		if answer.is_none() {
			unanswered.push(program.ticket.clone());
		}
		tickets.push(TicketRecord {
			ticket: program.ticket.clone(),
			answered: answer.is_some(),
			take: answer.map(|answer| answer.take.clone()),
			recorded_at: answer.map(|answer| answer.recorded_at.clone()),
			opens: opened,
		});
	}
	let closed = rust_round_repo::instant_order(&now(), &window.due) == Some(std::cmp::Ordering::Greater);
	Ok(Json(WeekRecord {
		week: week.id,
		opens: window.opens,
		due: window.due,
		closed,
		sent: sent.map(|held| SentAt {
			at: held.sent.at,
			received_at: held.received_at,
		}),
		tickets,
		unanswered,
		pastes,
	}))
}

/// `PUT /rust-round-log/sessions/:id`, the body being one session of the
/// phone's card-era log, as it holds it.
///
/// # Errors
/// 401 without a session, 403 for a browser's or from an untrusted origin,
/// 422 naming each refused field, 409 for a session that drops or changes
/// what home holds of it, 413 for a body longer than one byte past
/// `SESSION_CEILING` (the route's `DefaultBodyLimit`), 500 for a storage
/// failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "rust_round_log_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_session(paired: Paired, State(db): State<SqlitePool>, Path(id): Path<String>, body: Bytes) -> Result<Response, FileHostError> {
	let session = rust_round_repo::log::read_session(&body, &id).map_err(FileHostError::unprocessable_entity)?;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let (change, revision) = rust_round_repo::log::put_session(&mut tx, paired.subject.as_str(), &session, &now())
		.await
		.map_err(|err| refused(err, SESSION_REWRITTEN))?;
	tx.commit().await?;
	let status = if change == LogChange::Unchanged { StatusCode::OK } else { StatusCode::CREATED };
	let written = SessionWritten {
		change: change.as_str(),
		id: session.id,
		revision,
	};
	Ok((status, Json(written)).into_response())
}

/// `GET /rust-round-log/sessions`: every session the caller's subject holds,
/// each by its last revision, in the order they started.
///
/// # Errors
/// 401 without a session, 403 for a browser's, 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "rust_round_log_list", skip_all, fields(otel.kind = "server"))]
pub async fn list_sessions(paired: Paired, State(db): State<SqlitePool>) -> Result<Json<SessionList>, FileHostError> {
	let mut conn = db.acquire().await?;
	let sessions = rust_round_repo::log::sessions(&mut conn, paired.subject.as_str()).await?;
	Ok(Json(SessionList { sessions }))
}

/// `GET /rust-round-log/sessions/:id`: the session's last revision, byte for
/// byte as the phone sent it, with its hash as the `ETag`.
///
/// # Errors
/// 401 without a session, 403 for a browser's, 404 for a session the
/// caller's subject does not hold, 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "rust_round_log_get", skip_all, fields(otel.kind = "server"))]
pub async fn get_session(paired: Paired, State(db): State<SqlitePool>, headers: HeaderMap, Path(id): Path<String>) -> Result<Response, FileHostError> {
	let mut conn = db.acquire().await?;
	let body = rust_round_repo::log::session(&mut conn, paired.subject.as_str(), &id)
		.await?
		.ok_or(FileHostError::NotFound)?;
	tagged(&headers, body.into_bytes())
}
