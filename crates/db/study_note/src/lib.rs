//! The owner's study notes (paulgsc/some-ui#2180, HF5): the text the phone
//! keeps beside a soundbite or a `LeetType` spoken answer, kept on home so the
//! phone lets its copy go only once home has read it back.
//!
//! `20261101000600_create_study_note.up.sql` records the design; in short:
//!
//! - **Append-only, and kept for good.** [`put`] stores a note under its id,
//!   or nothing when that id already holds the same bytes; other bytes under
//!   a held id are refused ([`NoteError::Rewritten`]), never stored. No row
//!   is ever updated (a trigger aborts an `UPDATE`), and nothing here
//!   deletes. Account deletion is the one delete: its generic sweep of
//!   `SUBJECT_SCOPED_TABLES`, and the migration's trigger on `account`.
//! - **Content only, never parsed.** A body is stored byte for byte, but for
//!   the JSON whitespace around the value, after one check ([`validate`]):
//!   it is a JSON object within [`BODY_CEILING`]. What a note says stays the
//!   phone's to check, as `korean_repo`'s entries do.
//! - **A closed list of kinds** ([`Kind`]), one per store the phone keeps,
//!   so the record holds nothing the phone's stores do not.
//! - **Per subject, and only per subject.** Every query takes the subject and
//!   filters on it; nothing reads across subjects.
//! - **No background writer.** The one caller of [`put`] is `file_host`'s
//!   `handlers::study_note` (`PUT /api/v1/study-notes/:kind/:id`). A
//!   `git grep -n study_note -- '*.rs'` shows no other writer.

use curriculum_repo::content_hash;
use serde::Serialize;
use sqlx::SqliteConnection;

/// The most bytes one note's body may hold. An answer's note runs to a few
/// hundred bytes and a typed answer to a few KB; a soundbite's record is
/// smaller still.
pub const BODY_CEILING: usize = 64 * 1024;

/// The longest id the phone gives a note: a UUID is 36.
const MAX_ID_CHARS: usize = 64;

/// What JSON counts as whitespace between tokens (RFC 8259, section 2).
const JSON_WHITESPACE: [char; 4] = [' ', '\t', '\n', '\r'];

/// Which of the phone's stores a note is from (paulgsc/some-ui
/// `@some-ui/soundbites` and `apps/www/src/lib/leetype-takes`). Closed:
/// another kind is a reviewed diff here that says which store it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
	/// A soundbite's record, keyed by the soundbite's id; its audio is the
	/// voice note of that id, said for `soundbite`.
	Soundbite,
	/// A `LeetType` spoken answer's note, keyed by the note's own id; it names
	/// its take, if any, by the voice note's id, said for `leetype`.
	Leetype,
}

impl Kind {
	pub const ALL: [Self; 2] = [Self::Soundbite, Self::Leetype];

	/// The wire and stored name, and the `:kind` path segment.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Soundbite => "soundbite",
			Self::Leetype => "leetype",
		}
	}

	/// The kind `name` names, if any.
	#[must_use]
	pub fn named(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|kind| kind.as_str() == name)
	}
}

/// `(field, why)`: one reason a write was refused.
pub type Problem = (&'static str, &'static str);

/// A note as home holds it: what the phone compares with what it sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Note {
	pub kind: Kind,
	pub id: String,
	/// How many bytes home keeps: the value's own text.
	pub size: i64,
	/// Lowercase hex SHA-256 over those bytes.
	pub sha256: String,
}

/// What a put did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
	/// Home did not hold it: it is stored now.
	Added,
	/// Home already holds these very bytes: nothing written.
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
pub enum NoteError {
	/// The kind, id or body failed [`validate`]; every problem, by field.
	Invalid(Vec<Problem>),
	/// Home holds other bytes under this id: a note is never replaced.
	Rewritten,
	Storage(sqlx::Error),
}

/// Why a rewrite was refused, as the `409` says it.
pub const REWRITTEN: &str = "home already holds another note under this id";

impl std::fmt::Display for NoteError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Invalid(problems) => {
				f.write_str("not a note the record takes:")?;
				for (field, why) in problems {
					write!(f, " {field} {why};")?;
				}
				Ok(())
			}
			Self::Rewritten => f.write_str(REWRITTEN),
			Self::Storage(err) => write!(f, "database error: {err}"),
		}
	}
}

impl std::error::Error for NoteError {}

impl From<sqlx::Error> for NoteError {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

fn is_id(id: &str) -> bool {
	!id.is_empty() && id.chars().count() <= MAX_ID_CHARS && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn check_body(body: &[u8]) -> Result<&str, &'static str> {
	if body.len() > BODY_CEILING {
		return Err("is over the 65536-byte ceiling");
	}
	let text = std::str::from_utf8(body).map_err(|_| "is not UTF-8")?;
	let value = text.trim_matches(JSON_WHITESPACE);
	if !value.starts_with('{') {
		return Err("must be a JSON object");
	}
	serde_json::from_str::<serde::de::IgnoredAny>(value).map_err(|_| "is not JSON")?;
	Ok(value)
}

/// Checks a write's kind, id and body.
///
/// Answers the kind and the bytes kept: the JSON value's own text, the
/// whitespace around it trimmed. Every problem is reported, by field, so
/// the phone can say which note home refused.
///
/// # Errors
/// Each problem, by field: `kind`, `id`, `body`.
pub fn validate<'a>(kind: &str, id: &str, body: &'a [u8]) -> Result<(Kind, &'a str), Vec<Problem>> {
	let named = Kind::named(kind);
	let mut problems = Vec::new();
	if named.is_none() {
		problems.push(("kind", "is soundbite or leetype"));
	}
	if !is_id(id) {
		problems.push(("id", "is 1 to 64 letters, digits, '-', '_', '.' or ':'"));
	}
	let text = check_body(body).map_err(|why| problems.push(("body", why))).ok();
	match (named, text) {
		(Some(kind), Some(text)) if problems.is_empty() => Ok((kind, text)),
		_ => Err(problems),
	}
}

/// Stores `body` as `kind`'s note `id` for `subject`, as of `now`, and
/// answers what it did with the note as home now holds it.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the read that decides and
/// the write see one snapshot.
///
/// # Errors
/// [`NoteError::Invalid`] naming every problem [`validate`] finds, decided
/// before anything is written; [`NoteError::Rewritten`] for other bytes
/// under a held id; [`NoteError::Storage`] for any `sqlx` failure.
pub async fn put(conn: &mut SqliteConnection, subject: &str, kind: &str, id: &str, body: &[u8], now: &str) -> Result<(Change, Note), NoteError> {
	let (kind, text) = validate(kind, id, body).map_err(NoteError::Invalid)?;
	let sha256 = content_hash(text.as_bytes());
	let name = kind.as_str();
	let held = sqlx::query_scalar!("SELECT sha256 FROM study_note WHERE subject_id = ?1 AND kind = ?2 AND note_id = ?3", subject, name, id)
		.fetch_optional(&mut *conn)
		.await?;
	let note = Note {
		kind,
		id: id.to_owned(),
		size: i64::try_from(text.len()).unwrap_or(i64::MAX),
		sha256,
	};
	if let Some(held) = held {
		return if held == note.sha256 {
			Ok((Change::Unchanged, note))
		} else {
			Err(NoteError::Rewritten)
		};
	}
	sqlx::query!(
		"INSERT INTO study_note (subject_id, kind, note_id, body, sha256, stored_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
		subject,
		name,
		id,
		text,
		note.sha256,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok((Change::Added, note))
}

#[cfg(test)]
mod tests {
	use super::{put, validate, Change, Kind, NoteError, BODY_CEILING};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	/// The migration before `20261101000600_create_study_note`.
	const PREVIOUS_MIGRATION: i64 = 20_261_101_000_500;
	const T0: &str = "2026-11-01T07:00:00.000Z";
	const T1: &str = "2026-11-01T12:00:00.000Z";
	const A: &str = "subject-a";
	const B: &str = "subject-b";
	const ANSWER: &[u8] =
		br#"{"id":"n1","file":"two-sum.json","screen":"explain-1","prompt":"Why a map?","rubric":["O(1) lookup"],"ticks":["O(1) lookup"],"typed":null,"take":"y1","notedAt":1}"#;

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	async fn write(pool: &SqlitePool, subject: &str, kind: &str, id: &str, body: &[u8], now: &str) -> Result<Change, NoteError> {
		put(&mut pool.acquire().await.unwrap(), subject, kind, id, body, now).await.map(|(change, _)| change)
	}

	async fn rows(pool: &SqlitePool) -> i64 {
		sqlx::query_scalar("SELECT COUNT(*) FROM study_note").fetch_one(pool).await.unwrap()
	}

	#[tokio::test]
	async fn the_migration_round_trips_and_refuses_to_drop_a_record() {
		let pool = pool().await;
		let tables = || sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'study_note'");
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 1);
		MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 0, "down drops an empty table");
		MIGRATOR.run(&pool).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 1, "up recreates it");

		write(&pool, A, "leetype", "n1", ANSWER, T0).await.unwrap();
		assert!(MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.is_err(), "down refuses once the record holds a row");
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 1, "and leaves it as it was");
		assert_eq!(rows(&pool).await, 1);
	}

	/// The columns the migration describes and no others: a new one is a
	/// decision about what the server learns from the record.
	#[tokio::test]
	async fn the_table_holds_exactly_its_described_columns() {
		let pool = pool().await;
		let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('study_note') ORDER BY cid")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(columns, ["subject_id", "kind", "note_id", "body", "sha256", "stored_at"]);
	}

	/// A note is stored once: the same bytes again write nothing, other bytes
	/// under its id are refused, and no row is ever updated.
	#[tokio::test]
	async fn a_note_is_stored_once_and_never_replaced() {
		let pool = pool().await;
		assert_eq!(write(&pool, A, "leetype", "n1", ANSWER, T0).await.unwrap(), Change::Added);
		assert_eq!(write(&pool, A, "leetype", "n1", ANSWER, T1).await.unwrap(), Change::Unchanged, "a retry writes nothing");
		assert!(matches!(write(&pool, A, "leetype", "n1", br#"{"ticks":[]}"#, T1).await, Err(NoteError::Rewritten)));
		assert_eq!(write(&pool, A, "soundbite", "n1", br#"{"id":"n1"}"#, T1).await.unwrap(), Change::Added, "kinds stand apart");
		assert_eq!(write(&pool, B, "leetype", "n1", br#"{"other":true}"#, T1).await.unwrap(), Change::Added, "B's ids are B's");
		assert_eq!(rows(&pool).await, 3);
		let stored: String = sqlx::query_scalar("SELECT stored_at FROM study_note WHERE subject_id = ?1 AND kind = 'leetype'")
			.bind(A)
			.fetch_one(&pool)
			.await
			.unwrap();
		assert_eq!(stored, T0, "the first write stands");

		let update = sqlx::query("UPDATE study_note SET body = '{}'").execute(&pool).await;
		assert!(update.unwrap_err().to_string().contains("append-only"), "no row is ever updated");
	}

	#[tokio::test]
	async fn the_body_kept_is_the_bytes_sent_verbatim() {
		let pool = pool().await;
		let padded = b" \n{ \"ticks\" : [ \"a\" ],\"typed\":null }\r\n\t";
		let sent = "{ \"ticks\" : [ \"a\" ],\"typed\":null }";
		let (change, note) = put(&mut pool.acquire().await.unwrap(), A, "leetype", "n1", padded, T0).await.unwrap();
		assert_eq!((change, note.kind, note.size), (Change::Added, Kind::Leetype, i64::try_from(sent.len()).unwrap()));
		assert_eq!(note.sha256, curriculum_repo::content_hash(sent.as_bytes()), "the hash is over the value's own text");
		let body: String = sqlx::query_scalar("SELECT body FROM study_note").fetch_one(&pool).await.unwrap();
		assert_eq!(body, sent, "byte for byte, never re-serialised");
		let (again, _) = put(&mut pool.acquire().await.unwrap(), A, "leetype", "n1", sent.as_bytes(), T1).await.unwrap();
		assert_eq!(again, Change::Unchanged, "so the same value, padded or not, is the same bytes");
	}

	#[tokio::test]
	async fn a_refused_write_names_each_field_and_writes_nothing() {
		let fields = |problems: Vec<(&'static str, &'static str)>| problems.into_iter().map(|(field, _)| field).collect::<Vec<_>>();
		assert_eq!(fields(validate("note", "n 1", b"[]").unwrap_err()), ["kind", "id", "body"]);
		assert_eq!(fields(validate("leetype", "", b"{}").unwrap_err()), ["id"]);
		assert_eq!(fields(validate("leetype", &"n".repeat(65), b"{}").unwrap_err()), ["id"]);
		assert_eq!(fields(validate("leetype", "n1", b"{ not json").unwrap_err()), ["body"]);
		assert_eq!(fields(validate("leetype", "n1", b"\"text\"").unwrap_err()), ["body"]);
		assert_eq!(fields(validate("leetype", "n1", &vec![b' '; BODY_CEILING + 1]).unwrap_err()), ["body"]);
		assert_eq!(fields(validate("soundbite", "n1", b"\xff").unwrap_err()), ["body"]);
		assert_eq!(validate("soundbite", "b-1", b"{}").unwrap(), (Kind::Soundbite, "{}"));

		let pool = pool().await;
		assert!(matches!(write(&pool, A, "words", "n1", b"{}", T0).await, Err(NoteError::Invalid(_))));
		assert_eq!(rows(&pool).await, 0);
	}

	#[tokio::test]
	async fn the_record_leaves_with_the_account() {
		let pool = pool().await;
		for subject in [A, B] {
			sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?2)")
				.bind(subject)
				.bind(subject.as_bytes())
				.execute(&pool)
				.await
				.unwrap();
			write(&pool, subject, "leetype", "n1", ANSWER, T0).await.unwrap();
		}
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		let left: Vec<String> = sqlx::query_scalar("SELECT subject_id FROM study_note").fetch_all(&pool).await.unwrap();
		assert_eq!(left, [B], "A's record went with A's account, B's stayed");
	}
}
