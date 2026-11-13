//! A week's send, a draft until the week's deadline and final at it, for
//! each kind of week home keeps sends of (RR22, paulgsc/some-ui#2832; IW4,
//! paulgsc/some-ui#2935).
//!
//! The rules are Rust Rounds' (`migrations/20261110000100_create_rust_week_draft_final.up.sql`),
//! and a week of mock interview keeps them unchanged
//! (`migrations/20261113003000_create_interview_week_draft_final.up.sql`), so
//! they are written here once, for any [`WeekSend`]:
//!
//! - **A send is named by its `at`, compared as an instant** ([`put_draft`]):
//!   a later one replaces the draft, an earlier one (a late retry) leaves it,
//!   the same value again changes nothing, and another body at the draft's
//!   moment is refused ([`RoundError::Rewritten`]).
//! - **Final at the deadline** ([`finalize`]): once home's clock is past
//!   `due`, the draft standing is written as the week's final and dropped,
//!   in the caller's transaction. Each kind's send check refuses a send once
//!   home's clock is past `due`, so the final is the same whenever it is
//!   written.
//! - **What stands** ([`standing`]): the final once there is one, else the
//!   draft, read in one statement.
//!
//! Each kind has its own pair of tables ([`Kind`]); only the table a
//! statement names differs between them, since `sqlx`'s checked queries take
//! their SQL as a literal. Every query takes the subject and filters on it.

use crate::{instant_order, RoundError};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sqlx::SqliteConnection;
use std::cmp::Ordering;

/// The kind of week a send answers, and so the tables it is kept in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
	/// A week of Rust Rounds: `rust_week_draft` and `rust_week_final`.
	RustRounds,
	/// A week of mock interview: `interview_week_draft` and
	/// `interview_week_final`.
	Interview,
}

/// A week's send as the phone sends it, checked: what the draft and the
/// final need of it.
pub trait WeekSend: Serialize + DeserializeOwned + PartialEq + Clone + Send + Sync {
	/// Which tables it is kept in.
	const KIND: Kind;
	/// The week's id.
	fn week(&self) -> &str;
	/// When Send was pressed: names the send within its week.
	fn at(&self) -> &str;
}

/// What [`put_draft`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftChange {
	/// The week's first draft.
	Added,
	/// A later send replaced the draft.
	Replaced,
	/// The draft is this send already.
	Unchanged,
	/// The draft is a later send; nothing was written.
	Superseded,
}

impl DraftChange {
	/// The word the route answers with.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Added => "added",
			Self::Replaced => "replaced",
			Self::Unchanged => "unchanged",
			Self::Superseded => "superseded",
		}
	}
}

/// A week's send, as stored: its draft or its final.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored<S> {
	pub sent: S,
	/// A draft, before the deadline; else the week's final.
	pub draft: bool,
	/// The body as stored, re-serialised from the send.
	pub text: String,
	pub received_at: String,
}

/// One row of a draft or a final.
struct Row {
	sent_at: String,
	body: String,
	received_at: String,
}

fn stored<S: WeekSend>(body: String, received_at: String, draft: bool) -> Result<Stored<S>, sqlx::Error> {
	let sent = serde_json::from_str(&body).map_err(|err| sqlx::Error::Decode(Box::new(err)))?;
	Ok(Stored {
		sent,
		draft,
		text: body,
		received_at,
	})
}

async fn draft_row(conn: &mut SqliteConnection, kind: Kind, subject: &str, week: &str) -> Result<Option<Row>, sqlx::Error> {
	match kind {
		Kind::RustRounds => {
			sqlx::query_as!(
				Row,
				"SELECT sent_at, body, received_at FROM rust_week_draft WHERE subject_id = ?1 AND week_id = ?2",
				subject,
				week
			)
			.fetch_optional(&mut *conn)
			.await
		}
		Kind::Interview => {
			sqlx::query_as!(
				Row,
				"SELECT sent_at, body, received_at FROM interview_week_draft WHERE subject_id = ?1 AND week_id = ?2",
				subject,
				week
			)
			.fetch_optional(&mut *conn)
			.await
		}
	}
}

/// Keep `sent` as `subject`'s draft of its week, checked by its kind's
/// reader with home's clock no later than the week's deadline; say what was
/// done, and the draft that stands after it.
///
/// A send is named by its `at`, compared as an instant: a later one replaces
/// the draft, an earlier one (a late retry) leaves it, and one at the
/// draft's moment is the same send only if it is the same value (both read
/// as `S`, so key order and spacing are no part of it).
///
/// The caller refuses the send instead when [`is_final`] says the week is
/// final, in the same transaction: a final is never written beside a draft.
///
/// # Errors
/// [`RoundError::Rewritten`] for another body at the draft's moment, which
/// writes nothing; [`RoundError::Storage`] for any `sqlx` failure. Run it in
/// a `BEGIN IMMEDIATE` transaction, with `now` read once its lock is held.
pub async fn put_draft<S: WeekSend>(conn: &mut SqliteConnection, subject: &str, sent: &S, now: &str) -> Result<(DraftChange, S), RoundError> {
	// Re-serialising a checked value: its own field names and the phone's
	// strings, nothing more.
	#[allow(clippy::disallowed_methods)]
	let text = serde_json::to_string(sent).map_err(|err| RoundError::Storage(sqlx::Error::Decode(Box::new(err))))?;
	let held = match draft_row(conn, S::KIND, subject, sent.week()).await? {
		Some(row) => Some(stored::<S>(row.body, row.received_at, true)?),
		None => None,
	};
	let change = match held {
		None => DraftChange::Added,
		Some(held) => match crate::time_order_key(sent.at()).cmp(&crate::time_order_key(held.sent.at())) {
			Ordering::Equal if held.sent == *sent => return Ok((DraftChange::Unchanged, held.sent)),
			Ordering::Equal => return Err(RoundError::Rewritten),
			Ordering::Less => return Ok((DraftChange::Superseded, held.sent)),
			Ordering::Greater => DraftChange::Replaced,
		},
	};
	let (week, at) = (sent.week(), sent.at());
	match S::KIND {
		Kind::RustRounds => {
			sqlx::query!(
				"INSERT INTO rust_week_draft (subject_id, week_id, sent_at, body, received_at) VALUES (?1, ?2, ?3, ?4, ?5)
				 ON CONFLICT (subject_id, week_id) DO UPDATE SET sent_at = excluded.sent_at, body = excluded.body, received_at = excluded.received_at",
				subject,
				week,
				at,
				text,
				now
			)
			.execute(&mut *conn)
			.await?
		}
		Kind::Interview => {
			sqlx::query!(
				"INSERT INTO interview_week_draft (subject_id, week_id, sent_at, body, received_at) VALUES (?1, ?2, ?3, ?4, ?5)
				 ON CONFLICT (subject_id, week_id) DO UPDATE SET sent_at = excluded.sent_at, body = excluded.body, received_at = excluded.received_at",
				subject,
				week,
				at,
				text,
				now
			)
			.execute(&mut *conn)
			.await?
		}
	};
	Ok((change, sent.clone()))
}

/// Whether `week` of `kind` is final for `subject`: home holds its final.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn is_final(conn: &mut SqliteConnection, kind: Kind, subject: &str, week: &str) -> Result<bool, sqlx::Error> {
	match kind {
		Kind::RustRounds => {
			sqlx::query_scalar!(
				r#"SELECT EXISTS (SELECT 1 FROM rust_week_final WHERE subject_id = ?1 AND week_id = ?2) AS "held!: bool""#,
				subject,
				week
			)
			.fetch_one(&mut *conn)
			.await
		}
		Kind::Interview => {
			sqlx::query_scalar!(
				r#"SELECT EXISTS (SELECT 1 FROM interview_week_final WHERE subject_id = ?1 AND week_id = ?2) AS "held!: bool""#,
				subject,
				week
			)
			.fetch_one(&mut *conn)
			.await
		}
	}
}

/// `subject`'s send of `week` that stands: its final once it has one, else
/// its draft, if home holds either. Run [`finalize`] first once the week's
/// deadline has passed.
///
/// Both are read in one statement, so one snapshot: a finalization between
/// two reads could otherwise move the send out of the draft after the final
/// was found missing, and the week would read as never sent.
///
/// # Errors
/// Any `sqlx` failure, or a stored body that no longer reads as one.
pub async fn standing<S: WeekSend>(conn: &mut SqliteConnection, subject: &str, week: &str) -> Result<Option<Stored<S>>, sqlx::Error> {
	struct Standing {
		body: String,
		received_at: String,
		draft: bool,
	}
	let row = match S::KIND {
		Kind::RustRounds => {
			sqlx::query_as!(
				Standing,
				r#"SELECT body AS "body!", received_at AS "received_at!", draft AS "draft!: bool" FROM (
				       SELECT body, received_at, 0 AS draft FROM rust_week_final WHERE subject_id = ?1 AND week_id = ?2
				       UNION ALL
				       SELECT body, received_at, 1 AS draft FROM rust_week_draft WHERE subject_id = ?1 AND week_id = ?2)
				   ORDER BY draft LIMIT 1"#,
				subject,
				week
			)
			.fetch_optional(&mut *conn)
			.await?
		}
		Kind::Interview => {
			sqlx::query_as!(
				Standing,
				r#"SELECT body AS "body!", received_at AS "received_at!", draft AS "draft!: bool" FROM (
				       SELECT body, received_at, 0 AS draft FROM interview_week_final WHERE subject_id = ?1 AND week_id = ?2
				       UNION ALL
				       SELECT body, received_at, 1 AS draft FROM interview_week_draft WHERE subject_id = ?1 AND week_id = ?2)
				   ORDER BY draft LIMIT 1"#,
				subject,
				week
			)
			.fetch_optional(&mut *conn)
			.await?
		}
	};
	row.map(|row| stored(row.body, row.received_at, row.draft)).transpose()
}

/// The weeks of `kind` `subject` holds a draft of: one row per week not yet
/// final, by the key's own range.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn drafted_weeks(conn: &mut SqliteConnection, kind: Kind, subject: &str) -> Result<Vec<String>, sqlx::Error> {
	match kind {
		Kind::RustRounds => {
			sqlx::query_scalar!("SELECT week_id FROM rust_week_draft WHERE subject_id = ?1", subject)
				.fetch_all(&mut *conn)
				.await
		}
		Kind::Interview => {
			sqlx::query_scalar!("SELECT week_id FROM interview_week_draft WHERE subject_id = ?1", subject)
				.fetch_all(&mut *conn)
				.await
		}
	}
}

/// Make `week`'s standing draft its final once home's clock is past `due`;
/// whether it wrote one.
///
/// It writes the draft to the kind's final table as it stands and drops the
/// draft, in the caller's transaction. Nothing happens before `due`, nor
/// without a draft. Whenever it runs after `due` it writes the same final,
/// since no send is taken after it.
///
/// # Errors
/// Any `sqlx` failure. Run it in a `BEGIN IMMEDIATE` transaction.
pub async fn finalize(conn: &mut SqliteConnection, kind: Kind, subject: &str, week: &str, due: &str, now: &str) -> Result<bool, sqlx::Error> {
	if instant_order(now, due) != Some(Ordering::Greater) {
		return Ok(false);
	}
	let Some(held) = draft_row(conn, kind, subject, week).await? else {
		return Ok(false);
	};
	// A final is kept for good and read on every record, so a draft that no
	// longer reads as its kind's send is left a draft and nothing is written:
	// not an error, so a caller finalizing every due week (My data's kept
	// count) still answers. Its readers say it, as a draft past `due`
	// (`mailbox::index`'s `draft_after_due`) or a body that does not read.
	let reads = match kind {
		Kind::RustRounds => serde_json::from_str::<crate::sent::Sent>(&held.body).is_ok(),
		Kind::Interview => serde_json::from_str::<crate::interview::Sent>(&held.body).is_ok(),
	};
	if !reads {
		return Ok(false);
	}
	// The draft's `sent_at` is its send's `at`, as `put_draft` wrote it.
	if instant_order(&held.sent_at, due) == Some(Ordering::Greater) {
		// Dropped only once it is the final: a draft nothing could make final
		// (sent after `due`, which no send can make) is left for a person to
		// read, not lost.
		return Ok(false);
	}
	let written = match kind {
		Kind::RustRounds => {
			sqlx::query!(
				"INSERT INTO rust_week_final (subject_id, week_id, sent_at, body, received_at) VALUES (?1, ?2, ?3, ?4, ?5)
				 ON CONFLICT (subject_id, week_id) DO NOTHING",
				subject,
				week,
				held.sent_at,
				held.body,
				held.received_at
			)
			.execute(&mut *conn)
			.await?
		}
		Kind::Interview => {
			sqlx::query!(
				"INSERT INTO interview_week_final (subject_id, week_id, sent_at, body, received_at) VALUES (?1, ?2, ?3, ?4, ?5)
				 ON CONFLICT (subject_id, week_id) DO NOTHING",
				subject,
				week,
				held.sent_at,
				held.body,
				held.received_at
			)
			.execute(&mut *conn)
			.await?
		}
	}
	.rows_affected()
		== 1;
	// A draft a final already stands beside, which no send can make, is left
	// as it is for the same reason.
	if written {
		match kind {
			Kind::RustRounds => {
				sqlx::query!("DELETE FROM rust_week_draft WHERE subject_id = ?1 AND week_id = ?2", subject, week)
					.execute(&mut *conn)
					.await?
			}
			Kind::Interview => {
				sqlx::query!("DELETE FROM interview_week_draft WHERE subject_id = ?1 AND week_id = ?2", subject, week)
					.execute(&mut *conn)
					.await?
			}
		};
	}
	Ok(written)
}
