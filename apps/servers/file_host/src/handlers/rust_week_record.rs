//! Handlers for a week's record and the card-era log (RR14).
//!
//! These are `routes::rust_rounds`' validate-and-answer half
//! (paulgsc/some-ui#2689). What a week sent, a paste and a session must be, and the store's
//! rules (a draft until the deadline and final at it (RR22), append-only, a session grows by
//! revisions), are
//! `rust_round_repo`'s `sent` and `log`.
//!
//! **A week's record is read, never stored whole.** It is the week (the
//! routine's, in the mailbox), what the owner sent of it (the draft before
//! the deadline, the final after it), the errors pasted
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
use rust_round_repo::sent::DraftChange;
use rust_round_repo::sent::{Answer, Paste, Sent, StoredPaste, PASTED_TWICE, SENT_TWICE};
use rust_round_repo::week::Week;
use rust_round_repo::{Change, RoundError};
use serde::Serialize;
use sqlx::{Connection, SqliteConnection, SqlitePool};
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
	/// `added`, `replaced`, `unchanged` or `superseded`.
	pub change: &'static str,
	/// The week's draft as home keeps it after this send: this send, or for
	/// `superseded` the later one that stands.
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
	/// The week's send that stands: its draft before the deadline, its final
	/// after it, or null when home holds none (after the deadline, nothing
	/// was sent by it).
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
	/// A draft, which a later send before the deadline replaces; false for
	/// the week's final.
	pub draft: bool,
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

/// `PUT /rust-weeks/:id/sent`, the body being the phone's `WeekSent`.
///
/// The week's draft until its deadline, in a `BEGIN IMMEDIATE` transaction.
/// A send refused because the week is final makes it final first.
///
/// # Errors
/// 401 without a session, 403 for a browser's or from an untrusted origin,
/// 404 for a week the caller's subject does not hold, 422 naming each
/// refused field (`at` after the deadline, and `week` once home's clock is
/// past it, among them), 409 for another body at the draft's moment, 500 for
/// a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "rust_week_sent_put", skip_all, fields(otel.kind = "server"))]
pub async fn put_sent(paired: Paired, State(db): State<SqlitePool>, Path(id): Path<String>, body: Bytes) -> Result<Response, FileHostError> {
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	// Read once the write lock is held: a finalization that committed while
	// this send waited for it did so after the deadline, so this `now` is too,
	// and the send is refused rather than drafted beside the final.
	let now = now();
	let week = held_week(&mut tx, paired.subject.as_str(), &id).await?;
	if rust_round_repo::sent::is_final(&mut tx, paired.subject.as_str(), &week.id).await? {
		return Err(FileHostError::unprocessable_entity(vec![(
			String::from("week"),
			"is final: its deadline has passed, and what stood then is the record",
		)]));
	}
	let sent = match rust_round_repo::sent::read_sent(&body, &week, &now) {
		Ok(sent) => sent,
		Err(problems) => {
			finalize(&mut tx, paired.subject.as_str(), &week, &now).await?;
			tx.commit().await?;
			return Err(FileHostError::unprocessable_entity(problems));
		}
	};
	let (change, sent) = rust_round_repo::sent::put_draft(&mut tx, paired.subject.as_str(), &sent, &now)
		.await
		.map_err(|err| refused(err, SENT_TWICE))?;
	tx.commit().await?;
	let status = if change == DraftChange::Added { StatusCode::CREATED } else { StatusCode::OK };
	Ok((status, Json(SentWritten { change: change.as_str(), sent })).into_response())
}

/// Make `week` final for `subject` when home's clock is past its deadline
/// (`rust_round_repo::sent::finalize`), in the caller's transaction.
async fn finalize(conn: &mut SqliteConnection, subject: &str, week: &Week, now: &str) -> Result<(), FileHostError> {
	if let Some(window) = &week.window {
		rust_round_repo::sent::finalize(conn, subject, &week.id, &window.due, now).await?;
	}
	Ok(())
}

/// Make final every week `subject` holds a draft of whose deadline home's
/// clock has passed, in one `BEGIN IMMEDIATE` transaction: one row per week
/// not yet final, read by the draft table's key. A draft whose week home no
/// longer holds, or cannot read, is left as it is.
///
/// # Errors
/// 500 for a storage failure.
pub(crate) async fn finalize_due(db: &SqlitePool, subject: &str) -> Result<(), FileHostError> {
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let now = now();
	for id in rust_round_repo::sent::drafted_weeks(&mut tx, subject).await? {
		match held_week(&mut tx, subject, &id).await {
			Ok(week) => finalize(&mut tx, subject, &week, &now).await?,
			Err(FileHostError::NotFound) => {}
			Err(err) => return Err(err),
		}
	}
	tx.commit().await?;
	Ok(())
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
	let now = now();
	let mut conn = db.acquire().await?;
	let week = held_week(&mut conn, subject, &id).await?;
	let Some(window) = week.window.clone() else {
		tracing::error!(kind = "no_window", "a week home holds has no deadline it can read; no record");
		return Err(FileHostError::NotFound);
	};
	let (Some(from), Some(to)) = (stored_instant(&window.opens), stored_instant(&window.due)) else {
		return Err(FileHostError::NotFound);
	};
	let closed = rust_round_repo::instant_order(&now, &window.due) == Some(std::cmp::Ordering::Greater);
	if closed {
		let mut tx = conn.begin_with("BEGIN IMMEDIATE").await?;
		finalize(&mut tx, subject, &week, &now).await?;
		tx.commit().await?;
	}
	let sent = rust_round_repo::sent::standing(&mut conn, subject, &week.id).await?;
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
	Ok(Json(WeekRecord {
		week: week.id,
		opens: window.opens,
		due: window.due,
		closed,
		sent: sent.map(|held| SentAt {
			at: held.sent.at,
			received_at: held.received_at,
			draft: held.draft,
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
