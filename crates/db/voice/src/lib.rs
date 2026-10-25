//! The owner's voice notes (paulgsc/server#460), audio included: what the
//! phone sends home so home can keep each note and transcribe it.
//!
//! `20261025000100_create_voice_note.up.sql` records the design; in short:
//!
//! - **The one place home holds audio** (docs/identity.md invariant 23). A
//!   note is described in `voice_note` and its bytes sit beside it in
//!   `voice_note_audio`, written together in one transaction, so a read of
//!   what home holds never pulls audio.
//! - **Append-only, and kept for good.** [`put_note`] stores a note under its
//!   id, or nothing when that id already holds the same note; other bytes or
//!   another description under a held id are refused
//!   ([`VoiceError::Rewritten`]), never stored. No row is ever updated (the
//!   triggers abort an `UPDATE`), and nothing here deletes, a transcribed
//!   note included. Account deletion is the one delete: its generic sweep of
//!   `SUBJECT_SCOPED_TABLES`, and the migration's trigger on `account`.
//! - **Checked before it is kept.** A write passes [`describe`]: an id, what
//!   the note was said for, when, a container the transcriber reads, and a
//!   size within [`NOTE_CEILING`].
//! - **Per subject, and only per subject.** Every query takes the subject and
//!   filters on it; nothing reads across subjects.
//! - **No background writer of a note.** The one caller of [`put_note`] is
//!   `file_host`'s `handlers::voice` (`PUT /api/v1/voice-notes/:id`). A
//!   `git grep -n 'voice_note\b\|voice_note_audio' -- '*.rs'` shows no
//!   other writer.
//!
//! What home heard in each note is [`transcript`]'s: `voice_transcript`,
//! written only by `file_host`'s `transcribe` worker, which reads a note's
//! audio and never writes it.

pub mod transcript;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use sqlx::SqliteConnection;

/// A field and why it was refused.
pub type Problem = (&'static str, &'static str);

/// The most bytes one note may hold.
///
/// About four minutes at the phone's 32 kbps: four times its own one-minute
/// cap (`SOUNDBITE_TAKE_MAX_BYTES` in paulgsc/some-ui, 240,000 bytes), and
/// under axum's default body limit.
pub const NOTE_CEILING: usize = 1024 * 1024;

const MAX_ID_CHARS: usize = 64;
const MAX_REF_CHARS: usize = 200;

/// What a note was said for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Of {
	/// A study session's reflection.
	Reflection,
	/// A Jobs pick or scout.
	Jobs,
	/// A drama prompt's next scene, said aloud.
	Scene,
}

impl Of {
	const ALL: [Self; 3] = [Self::Reflection, Self::Jobs, Self::Scene];

	/// The wire and stored name.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Reflection => "reflection",
			Self::Jobs => "jobs",
			Self::Scene => "scene",
		}
	}

	/// The kind `name` names, if any.
	#[must_use]
	pub fn named(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|of| of.as_str() == name)
	}
}

/// The containers the phone records in, and the transcriber reads.
pub const CONTENT_TYPES: [&str; 3] = ["audio/webm", "audio/ogg", "audio/mp4"];

/// A note as home describes it: everything but its bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
	/// The phone's id for the note.
	pub id: String,
	/// What it was said for.
	pub of: Of,
	/// Which one: the session's id, the pick's or scout's id, or the drama
	/// prompt's stamp, as the phone names it.
	#[serde(rename = "ref")]
	pub of_ref: String,
	/// When the phone recorded it, as ISO-8601 UTC to the millisecond.
	pub at: String,
	/// Its container, one of [`CONTENT_TYPES`].
	pub content_type: &'static str,
	/// Its length in bytes.
	pub size: i64,
	/// The lowercase hex SHA-256 of its bytes.
	pub sha256: String,
}

/// What the phone says about a note it sends, before home checks it.
#[derive(Debug, Clone, Copy)]
pub struct Said<'a> {
	pub of: Option<&'a str>,
	pub of_ref: Option<&'a str>,
	pub at: Option<&'a str>,
	/// The request's `Content-Type`, parameters and all.
	pub content_type: Option<&'a str>,
}

/// `at` as ISO-8601 UTC to the millisecond, if it is an RFC 3339 instant.
fn instant(at: &str) -> Option<String> {
	DateTime::parse_from_rfc3339(at)
		.ok()
		.map(|at| at.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Millis, true))
}

/// A content type's container without its parameters (`audio/webm;codecs=opus`
/// is `audio/webm`), if it is one the transcriber reads.
fn container(content_type: &str) -> Option<&'static str> {
	let base = content_type.split(';').next().unwrap_or_default().trim();
	CONTENT_TYPES.into_iter().find(|known| known.eq_ignore_ascii_case(base))
}

/// Whether `text` is a non-empty name of at most `max` characters, each a
/// letter, digit, `-`, `_`, `.` or `:`, as the phone's ids and stamps are.
fn is_name(text: &str, max: usize) -> bool {
	!text.is_empty() && text.chars().count() <= max && text.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

/// The note `body` is, as the phone `said`, under id `id`; or every problem
/// with it, by field.
///
/// # Errors
/// Every refused field: `id`, `of`, `ref`, `at`, `contentType` or `body`.
pub fn describe(id: &str, said: Said<'_>, body: &[u8]) -> Result<Note, Vec<Problem>> {
	let mut problems: Vec<Problem> = Vec::new();
	if !is_name(id, MAX_ID_CHARS) {
		problems.push(("id", "not a note id: 1 to 64 letters, digits, '-', '_', '.' or ':'"));
	}
	let of = said.of.and_then(Of::named);
	if of.is_none() {
		problems.push(("of", "not what a note is said for: reflection, jobs or scene"));
	}
	let of_ref = said.of_ref.filter(|of_ref| is_name(of_ref, MAX_REF_CHARS));
	if of_ref.is_none() {
		problems.push(("ref", "not which one: 1 to 200 letters, digits, '-', '_', '.' or ':'"));
	}
	let at = said.at.and_then(instant);
	if at.is_none() {
		problems.push(("at", "not an RFC 3339 instant"));
	}
	let content_type = said.content_type.and_then(container);
	if content_type.is_none() {
		problems.push(("contentType", "not audio/webm, audio/ogg or audio/mp4"));
	}
	if body.is_empty() {
		problems.push(("body", "empty: a note holds audio"));
	} else if body.len() > NOTE_CEILING {
		problems.push(("body", "larger than a note may be (1 MiB)"));
	}
	let size = i64::try_from(body.len()).ok();
	match (of, of_ref, at, content_type, size) {
		(Some(of), Some(of_ref), Some(at), Some(content_type), Some(size)) if problems.is_empty() => Ok(Note {
			id: id.to_owned(),
			of,
			of_ref: of_ref.to_owned(),
			at,
			content_type,
			size,
			sha256: hex::encode(Sha256::digest(body)),
		}),
		_ => Err(problems),
	}
}

/// What a put did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
	/// Home did not hold it: it is stored now.
	Added,
	/// Home already holds this very note: nothing written.
	Unchanged,
}

impl Change {
	/// The wire name.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Added => "added",
			Self::Unchanged => "unchanged",
		}
	}
}

/// Why a put wrote nothing.
#[derive(Debug)]
pub enum VoiceError {
	/// The id already holds a different note, which is never replaced.
	Rewritten,
	/// A stored description this crate wrote does not read back.
	Unreadable,
	Storage(sqlx::Error),
}

impl std::fmt::Display for VoiceError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Rewritten => f.write_str(REWRITTEN),
			Self::Unreadable => f.write_str("a stored voice note does not read back"),
			Self::Storage(err) => write!(f, "database error: {err}"),
		}
	}
}

impl std::error::Error for VoiceError {}

impl From<sqlx::Error> for VoiceError {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

/// What [`VoiceError::Rewritten`] says.
pub const REWRITTEN: &str = "this note id already holds a different note: voice notes are append-only";

/// Stores `note`, whose bytes are `body`, for `subject` as of `now`: its
/// description and its audio together.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the read that decides and
/// both writes see one snapshot, and land together or not at all.
///
/// # Errors
/// [`VoiceError::Rewritten`] when the note's id already holds other bytes or
/// another description, [`VoiceError::Unreadable`] for a held description
/// that does not read back, and [`VoiceError::Storage`] for any `sqlx`
/// failure. A refusal is decided before anything is written.
pub async fn put_note(conn: &mut SqliteConnection, subject: &str, note: &Note, body: &[u8], now: &str) -> Result<Change, VoiceError> {
	let standing = sqlx::query!(
		r#"SELECT of_kind, of_ref, recorded_at, content_type, size, sha256 FROM voice_note WHERE subject_id = ?1 AND note_id = ?2"#,
		subject,
		note.id
	)
	.fetch_optional(&mut *conn)
	.await?;
	if let Some(standing) = standing {
		let held = Note {
			id: note.id.clone(),
			of: Of::named(&standing.of_kind).ok_or(VoiceError::Unreadable)?,
			of_ref: standing.of_ref,
			at: standing.recorded_at,
			content_type: container(&standing.content_type).ok_or(VoiceError::Unreadable)?,
			size: standing.size,
			sha256: standing.sha256,
		};
		return if held == *note { Ok(Change::Unchanged) } else { Err(VoiceError::Rewritten) };
	}
	let of = note.of.as_str();
	sqlx::query!(
		"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
		 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
		subject,
		note.id,
		of,
		note.of_ref,
		note.at,
		note.content_type,
		note.size,
		note.sha256,
		now
	)
	.execute(&mut *conn)
	.await?;
	sqlx::query!("INSERT INTO voice_note_audio (subject_id, note_id, body) VALUES (?1, ?2, ?3)", subject, note.id, body)
		.execute(&mut *conn)
		.await?;
	Ok(Change::Added)
}

#[cfg(test)]
mod tests {
	use super::{describe, put_note, Change, Note, Of, Said, VoiceError, NOTE_CEILING};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const PREVIOUS_MIGRATION: i64 = 20_261_023_000_200;
	const NOW: &str = "2026-10-25T03:00:00.000Z";
	const A: &str = "subject-a";
	const B: &str = "subject-b";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn said(at: &str) -> Said<'_> {
		Said {
			of: Some("reflection"),
			of_ref: Some("session-7f2a"),
			at: Some(at),
			content_type: Some("audio/webm;codecs=opus"),
		}
	}

	fn note(id: &str, body: &[u8]) -> Note {
		describe(id, said("2026-10-24T20:10:00-07:00"), body).unwrap()
	}

	async fn put(pool: &SqlitePool, subject: &str, note: &Note, body: &[u8]) -> Result<Change, VoiceError> {
		let mut tx = pool.begin().await.unwrap();
		let change = put_note(&mut tx, subject, note, body, NOW).await?;
		tx.commit().await.unwrap();
		Ok(change)
	}

	async fn rows(pool: &SqlitePool, table: &str) -> i64 {
		sqlx::query_scalar(&(String::from("SELECT COUNT(*) FROM ") + table)).fetch_one(pool).await.unwrap()
	}

	#[tokio::test]
	async fn the_migration_round_trips() {
		let pool = pool().await;
		let tables = || sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('voice_note', 'voice_note_audio')");
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 2);
		MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 0, "down drops them");
		MIGRATOR.run(&pool).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 2, "up recreates them");
	}

	/// The columns the migration describes and no others: a new one is a
	/// decision about what the server learns from a note.
	#[tokio::test]
	async fn the_tables_hold_exactly_their_described_columns() {
		let pool = pool().await;
		for (table, expected) in [
			(
				"voice_note",
				&["subject_id", "note_id", "of_kind", "of_ref", "recorded_at", "content_type", "size", "sha256", "stored_at"][..],
			),
			("voice_note_audio", &["subject_id", "note_id", "body"][..]),
		] {
			let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
				.bind(table)
				.fetch_all(&pool)
				.await
				.unwrap();
			assert_eq!(columns, expected, "{table}");
		}
	}

	#[test]
	fn a_note_is_described_as_home_keeps_it() {
		let body = b"opus frames";
		let held = note("n1", body);
		assert_eq!(held.of, Of::Reflection);
		assert_eq!(held.at, "2026-10-25T03:10:00.000Z", "the instant in UTC, to the millisecond");
		assert_eq!(held.content_type, "audio/webm", "the container, without its parameters");
		assert_eq!(held.size, 11);
		assert_eq!(held.sha256.len(), 64);
		assert!(held.sha256.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
	}

	#[test]
	fn a_refused_note_names_every_field() {
		let nothing = Said {
			of: None,
			of_ref: None,
			at: None,
			content_type: None,
		};
		let fields: Vec<&str> = describe("", nothing, b"").unwrap_err().into_iter().map(|(field, _)| field).collect();
		assert_eq!(fields, ["id", "of", "ref", "at", "contentType", "body"]);

		let wrong = Said {
			of: Some("diary"),
			of_ref: Some("has a space"),
			at: Some("yesterday"),
			content_type: Some("video/webm"),
		};
		let fields: Vec<&str> = describe("a/b", wrong, &vec![0; NOTE_CEILING + 1])
			.unwrap_err()
			.into_iter()
			.map(|(field, _)| field)
			.collect();
		assert_eq!(fields, ["id", "of", "ref", "at", "contentType", "body"]);

		for content_type in ["audio/webm", "AUDIO/OGG; codecs=opus", "audio/mp4"] {
			let ok = Said {
				content_type: Some(content_type),
				..said("2026-10-25T03:10:00Z")
			};
			assert!(describe("n1", ok, b"x").is_ok(), "{content_type}");
		}
		assert!(
			describe("n1", said("2026-10-25T03:10:00Z"), &vec![0; NOTE_CEILING]).is_ok(),
			"the ceiling itself is allowed"
		);
	}

	#[tokio::test]
	async fn a_note_is_stored_once_and_never_replaced() {
		let pool = pool().await;
		let first = note("n1", b"take one");
		assert_eq!(put(&pool, A, &first, b"take one").await.unwrap(), Change::Added);
		assert_eq!(put(&pool, A, &first, b"take one").await.unwrap(), Change::Unchanged, "the same note writes nothing");

		let other_bytes = note("n1", b"take two");
		assert!(
			matches!(put(&pool, A, &other_bytes, b"take two").await, Err(VoiceError::Rewritten)),
			"other bytes under a held id"
		);
		let mut other_ref = first.clone();
		other_ref.of_ref = String::from("session-other");
		assert!(
			matches!(put(&pool, A, &other_ref, b"take one").await, Err(VoiceError::Rewritten)),
			"another description under a held id"
		);
		assert_eq!((rows(&pool, "voice_note").await, rows(&pool, "voice_note_audio").await), (1, 1));

		assert_eq!(put(&pool, B, &other_bytes, b"take two").await.unwrap(), Change::Added, "B's ids are B's");

		for update in ["UPDATE voice_note SET of_ref = 'x'", "UPDATE voice_note_audio SET body = x'00'"] {
			let refused = sqlx::query(update).execute(&pool).await;
			assert!(refused.unwrap_err().to_string().contains("append-only"), "no row is ever updated: {update}");
		}
	}

	#[tokio::test]
	async fn the_audio_is_kept_byte_for_byte_beside_its_note() {
		let pool = pool().await;
		let body: Vec<u8> = (0..=255).collect();
		put(&pool, A, &note("n1", &body), &body).await.unwrap();
		let held: Vec<u8> = sqlx::query_scalar("SELECT body FROM voice_note_audio WHERE subject_id = ?1 AND note_id = 'n1'")
			.bind(A)
			.fetch_one(&pool)
			.await
			.unwrap();
		assert_eq!(held, body);
		let orphans: i64 = sqlx::query_scalar(
			"SELECT COUNT(*) FROM voice_note_audio a WHERE NOT EXISTS
			 (SELECT 1 FROM voice_note n WHERE n.subject_id = a.subject_id AND n.note_id = a.note_id)",
		)
		.fetch_one(&pool)
		.await
		.unwrap();
		assert_eq!(orphans, 0);
	}

	#[tokio::test]
	async fn a_note_whose_audio_fails_leaves_no_description() {
		let pool = pool().await;
		// Audio already held under the id with no description: the second
		// insert fails, and the transaction keeps neither half.
		sqlx::query("INSERT INTO voice_note_audio (subject_id, note_id, body) VALUES (?1, 'n1', x'00')")
			.bind(A)
			.execute(&pool)
			.await
			.unwrap();
		assert!(matches!(put(&pool, A, &note("n1", b"take"), b"take").await, Err(VoiceError::Storage(_))));
		assert_eq!(rows(&pool, "voice_note").await, 0);
	}

	#[tokio::test]
	async fn the_notes_leave_with_the_account() {
		let pool = pool().await;
		for subject in [A, B] {
			sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?2)")
				.bind(subject)
				.bind(subject.as_bytes())
				.execute(&pool)
				.await
				.unwrap();
			put(&pool, subject, &note("n1", b"take"), b"take").await.unwrap();
		}
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		for table in ["voice_note", "voice_note_audio"] {
			let left: Vec<String> = sqlx::query_scalar(&(String::from("SELECT subject_id FROM ") + table)).fetch_all(&pool).await.unwrap();
			assert_eq!(left, [B], "{table}: A's notes went with A's account, B's stayed");
		}
	}
}
