//! A conversation essay's record, kept on home (HOST16-S, paulgsc/server#1243;
//! paulgsc/some-ui C1535): one append-only log of `some-ui.conversation/2`
//! events per conversation, per subject, whose state is the log replayed.
//!
//! `20261126000100_create_conversation_event.up.sql` records the design; in
//! short:
//!
//! - **Append-only.** [`append`] keeps one event at the log's end, after
//!   replaying the log with the event through [`step`]; an event the step
//!   refuses is never kept ([`ConversationError::Refused`]). No row is ever
//!   updated (a trigger aborts an `UPDATE`), and nothing here deletes: the
//!   only delete is the account's.
//! - **A retried send keeps nothing twice.** An event equal to the log's last
//!   is answered as kept, at its index, and nothing is written
//!   ([`Change::Unchanged`]): a pick the phone sends again after a lost
//!   answer is not refused as a second approval.
//! - **Ordered by its number, never by a clock.** `seq` counts from 1 with no
//!   gap (a trigger holds it); each event's `at` is the writer's and only the
//!   step reads it.
//! - **Per subject, and only per subject.** Every query takes the subject and
//!   filters on it.
//! - **The one writer** is `file_host`'s `handlers::conversations`
//!   (`POST /api/v1/conversations/:id/events`). A
//!   `git grep -n conversation_event -- '*.rs'` shows no other.

pub mod step;

use serde::Serialize;
use serde_json::Value;
use sqlx::SqliteConnection;
use step::{Event, Refusal, State};

/// The largest event a write takes: a version of forty turns, each with its
/// evidence and a note of 4,000 characters, is well under it.
pub const BODY_CEILING: usize = 1024 * 1024;

/// What an append did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
	/// Kept now, at the log's end.
	Added,
	/// The log's last event is this very one: nothing written.
	Unchanged,
}

/// Where an event stands in its log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Kept {
	/// The event's index in the log, from 0.
	pub kept: usize,
	/// How many events the log holds.
	pub events: usize,
}

/// Why an append wrote nothing.
#[derive(Debug)]
pub enum ConversationError {
	/// The record's step refused it: the index it would have had, and why.
	Refused {
		at: usize,
		refusal: Refusal,
	},
	/// The log home keeps does not replay: a kept event is unreadable, or
	/// refused by the step. Nothing is appended to a log in this state.
	Unreplayable {
		at: usize,
	},
	Storage(sqlx::Error),
}

impl std::fmt::Display for ConversationError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Refused { at, .. } => write!(f, "the record's step refused event {at}"),
			Self::Unreplayable { at } => write!(f, "the kept log does not replay at event {at}"),
			Self::Storage(err) => write!(f, "database error: {err}"),
		}
	}
}

impl std::error::Error for ConversationError {}

impl From<sqlx::Error> for ConversationError {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

/// Each kept event of `conversation`, as sent, oldest first.
///
/// # Errors
/// Any `sqlx` failure; [`ConversationError::Unreplayable`] for a kept body
/// that is not JSON.
pub async fn values(conn: &mut SqliteConnection, subject: &str, conversation: &str) -> Result<Vec<Value>, ConversationError> {
	let rows = sqlx::query!(
		"SELECT seq, body FROM conversation_event WHERE subject_id = ?1 AND conversation_id = ?2 ORDER BY seq",
		subject,
		conversation
	)
	.fetch_all(&mut *conn)
	.await?;
	rows
		.into_iter()
		.enumerate()
		.map(|(at, row)| serde_json::from_str(&row.body).map_err(|err| unreadable(at, &err)))
		.collect()
}

fn unreadable(at: usize, err: &serde_json::Error) -> ConversationError {
	// A kept body home wrote itself: never what it says, only where.
	tracing::error!(at, line = err.line(), "a kept conversation event is not JSON");
	ConversationError::Unreplayable { at }
}

/// The record as `some-ui.conversation/2` keeps it, or `None` when home
/// keeps no event of `conversation`.
///
/// # Errors
/// As [`values`].
pub async fn record(conn: &mut SqliteConnection, subject: &str, conversation: &str) -> Result<Option<Value>, ConversationError> {
	let events = values(conn, subject, conversation).await?;
	if events.is_empty() {
		return Ok(None);
	}
	Ok(Some(serde_json::json!({ "schema": step::SCHEMA, "id": conversation, "events": events })))
}

/// The log replayed, from each kept event's value.
///
/// # Errors
/// [`ConversationError::Unreplayable`] at the first kept event that does not
/// read or that the step refuses.
pub fn replay(values: Vec<Value>) -> Result<(Vec<Event>, State), ConversationError> {
	let mut events = Vec::with_capacity(values.len());
	let mut state = State::new();
	for (at, value) in values.into_iter().enumerate() {
		let Ok(event) = step::parse(value) else {
			return Err(ConversationError::Unreplayable { at });
		};
		state = state.step(&event).map_err(|_refused| ConversationError::Unreplayable { at })?;
		events.push(event);
	}
	Ok((events, state))
}

/// The state `conversation`'s kept log leaves.
///
/// # Errors
/// As [`values`] and [`replay`].
pub async fn state(conn: &mut SqliteConnection, subject: &str, conversation: &str) -> Result<State, ConversationError> {
	let values = values(conn, subject, conversation).await?;
	Ok(replay(values)?.1)
}

/// What [`prepare`] found: the event is the log's last already, or it is
/// the next, which the step takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prepared {
	/// The log's last event is this very one: a retried send.
	Unchanged(Kept),
	/// The step takes it, as the log's next, at this index.
	Next(usize),
}

/// Whether `event` is kept already or would be the next of `conversation`'s
/// log for `subject`, which it replays: nothing is written.
///
/// Run it and [`insert`] in one `BEGIN IMMEDIATE` transaction, so the replay
/// that decides and the write see one snapshot.
///
/// # Errors
/// [`ConversationError::Refused`] when the step refuses `event`,
/// [`ConversationError::Unreplayable`] when the kept log does not replay, and
/// [`ConversationError::Storage`] for any `sqlx` failure.
pub async fn prepare(conn: &mut SqliteConnection, subject: &str, conversation: &str, event: &Event) -> Result<Prepared, ConversationError> {
	let values = values(conn, subject, conversation).await?;
	let count = values.len();
	if values.last() == Some(&event.value) {
		return Ok(Prepared::Unchanged(Kept { kept: count - 1, events: count }));
	}
	let (_, state) = replay(values)?;
	state.step(event).map_err(|refusal| ConversationError::Refused { at: count, refusal })?;
	Ok(Prepared::Next(count))
}

/// Writes `event` as the log's event `at` (from 0), as [`prepare`] found it,
/// as of `now`. The table's trigger refuses any but the next.
///
/// # Errors
/// [`ConversationError::Storage`] for any `sqlx` failure.
pub async fn insert(conn: &mut SqliteConnection, subject: &str, conversation: &str, event: &Event, at: usize, now: &str) -> Result<Kept, ConversationError> {
	let seq = i64::try_from(at + 1).map_err(|err| ConversationError::Storage(sqlx::Error::Decode(Box::new(err))))?;
	let body = event.value.to_string();
	let kind = event.kind.as_str();
	sqlx::query!(
		"INSERT INTO conversation_event (subject_id, conversation_id, seq, event, at, body, stored_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
		subject,
		conversation,
		seq,
		kind,
		event.at,
		body,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok(Kept { kept: at, events: at + 1 })
}

/// Keeps `event` at the end of `conversation`'s log for `subject`, as of
/// `now`, after replaying the log with it: [`prepare`], then [`insert`].
///
/// Run it in a `BEGIN IMMEDIATE` transaction. `conversation` is an id
/// (`step::is_id`).
///
/// # Errors
/// As [`prepare`] and [`insert`]. A refusal is decided before anything is
/// written.
pub async fn append(conn: &mut SqliteConnection, subject: &str, conversation: &str, event: &Event, now: &str) -> Result<(Change, Kept), ConversationError> {
	match prepare(conn, subject, conversation, event).await? {
		Prepared::Unchanged(kept) => Ok((Change::Unchanged, kept)),
		Prepared::Next(at) => Ok((Change::Added, insert(conn, subject, conversation, event, at, now).await?)),
	}
}

#[cfg(test)]
mod tests;
