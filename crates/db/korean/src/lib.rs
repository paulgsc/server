//! The owner's Korean record (paulgsc/some-ui#2100, KO-HOME): the word sets
//! the phone studies on and what it records as they are read, kept on home
//! so the phone keeps only what home has not confirmed and the offline set
//! its person curates.
//!
//! `20261026000200_create_korean_entry.up.sql` records the design; in short:
//!
//! - **Append-only.** [`put`] adds a revision, or nothing when its bytes
//!   equal the standing ones; [`remove`] adds a revision with no body, or
//!   nothing when nothing stands. No row is ever updated (a trigger aborts
//!   an `UPDATE`), and nothing here deletes. Account deletion is the one
//!   delete: its generic sweep of `SUBJECT_SCOPED_TABLES`, and the
//!   migration's trigger on `account`.
//! - **Content only, never parsed.** A body is stored byte for byte after one
//!   check ([`validate`]): it is a JSON object or array within
//!   [`BODY_CEILING`]. What a word set or a pace *is* stays the phone's to
//!   check, as the learner shelf's items do (canon Def. 8.3: the server
//!   never parses a lesson).
//! - **A closed list of kinds** ([`Kind`]), one per store the phone keeps,
//!   so the record holds nothing the phone's stores do not.
//! - **Per subject, and only per subject.** Every query takes the subject and
//!   filters on it; nothing reads across subjects.
//! - **No background writer.** The callers are `file_host`'s
//!   `handlers::korean` (`PUT` and `DELETE /api/v1/korean/:kind/:key`) and
//!   `handlers::sync` (`POST /api/v1/sync/korean/batch`, each item as one of
//!   those routes), which the phone sends once paired. A
//!   `git grep -n korean_entry -- '*.rs'` shows no other writer.

use curriculum_repo::{content_hash, is_plain_key};
use serde::Serialize;
use serde_json::value::RawValue;
use sqlx::SqliteConnection;

/// The most bytes one entry's body may hold: the learner shelf's ceiling.
/// A word set runs to tens of KB, and the single documents (`record`,
/// `progress`, `feedback`) to a few.
pub const BODY_CEILING: usize = 256 * 1024;

/// What JSON counts as whitespace between tokens (RFC 8259, section 2).
const JSON_WHITESPACE: [char; 4] = [' ', '\t', '\n', '\r'];

/// Which of the phone's stores an entry is from (paulgsc/some-ui
/// `packages/ui/topik/src/lib/topik/adapter`). Closed: another kind is a
/// reviewed diff here that says which store it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
	/// A word set added on the phone (`read-aloud-store`'s sets), keyed by the
	/// set's id.
	Set,
	/// One word's pace (the pace book), keyed by the word's id.
	Pace,
	/// One sentence's tally row, keyed by the phone's key for the sentence.
	Tally,
	/// One entry of the grammar lens's record (`lens-record-store`).
	Lens,
	/// The practice record: counts by day, shown to the learner.
	Record,
	/// The set left unfinished, where the drill resumes.
	Progress,
	/// The learner's two answers at a sitting's end.
	Feedback,
}

impl Kind {
	pub const ALL: [Self; 7] = [Self::Set, Self::Pace, Self::Tally, Self::Lens, Self::Record, Self::Progress, Self::Feedback];

	/// The wire and stored name, and the `:kind` path segment.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Set => "set",
			Self::Pace => "pace",
			Self::Tally => "tally",
			Self::Lens => "lens",
			Self::Record => "record",
			Self::Progress => "progress",
			Self::Feedback => "feedback",
		}
	}

	/// The kind `name` names, if any.
	#[must_use]
	pub fn named(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|kind| kind.as_str() == name)
	}
}

/// Why a kind was refused, as a write reports it.
pub const KIND_PROBLEM: &str = "is set, pace, tally, lens, record, progress or feedback";

/// Why a key was refused, as a write reports it.
pub const KEY_PROBLEM: &str = "must be a plain key: URL-unreserved characters, not starting with `.` or `http`, no `.json` suffix";

/// `(field, why)`: one reason a write was refused.
pub type Problem = (&'static str, &'static str);

/// One entry as it stands, or a revision that removed it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
	pub key: String,
	pub revision: i64,
	/// The bytes the phone sent, verbatim; `None` once removed.
	pub body: Option<Box<RawValue>>,
	/// ISO-8601 UTC, when this revision was written.
	pub written_at: String,
}

impl PartialEq for Entry {
	fn eq(&self, other: &Self) -> bool {
		self.key == other.key
			&& self.revision == other.revision
			&& self.body.as_ref().map(|body| body.get()) == other.body.as_ref().map(|body| body.get())
			&& self.written_at == other.written_at
	}
}

impl Eq for Entry {}

/// What a write did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
	/// The first revision: the record did not hold it.
	Kept,
	/// A new revision over a different standing one, or over a removal.
	Revised,
	/// A revision that removed what stood.
	Removed,
	/// The standing revision already says it: nothing written.
	Unchanged,
}

impl Change {
	/// The wire name.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Kept => "kept",
			Self::Revised => "revised",
			Self::Removed => "removed",
			Self::Unchanged => "unchanged",
		}
	}
}

/// Why a write wrote nothing.
#[derive(Debug)]
pub enum KoreanError {
	/// The kind, key or body failed [`validate`]; every problem, by field.
	Invalid(Vec<Problem>),
	/// A stored body this crate wrote does not read back as JSON.
	Unreadable,
	Storage(sqlx::Error),
}

impl std::fmt::Display for KoreanError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Invalid(problems) => {
				f.write_str("not a write the Korean record takes:")?;
				for (field, why) in problems {
					write!(f, " {field} {why};")?;
				}
				Ok(())
			}
			Self::Unreadable => f.write_str("a stored Korean revision does not read back"),
			Self::Storage(err) => write!(f, "database error: {err}"),
		}
	}
}

impl std::error::Error for KoreanError {}

impl From<sqlx::Error> for KoreanError {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

/// Checks a write's kind and key, and its body when it has one. Every
/// problem is reported, by field, so the phone can say which part home
/// refused.
///
/// # Errors
/// Each problem, by field: `kind`, `key`, `body`.
pub fn validate(kind: &str, key: &str, body: Option<&[u8]>) -> Result<Kind, Vec<Problem>> {
	let named = Kind::named(kind);
	let mut problems = Vec::new();
	if named.is_none() {
		problems.push(("kind", KIND_PROBLEM));
	}
	if !is_plain_key(key) {
		problems.push(("key", KEY_PROBLEM));
	}
	if let Some(Err(why)) = body.map(check_body) {
		problems.push(("body", why));
	}
	match named {
		Some(kind) if problems.is_empty() => Ok(kind),
		_ => Err(problems),
	}
}

fn check_body(body: &[u8]) -> Result<(), &'static str> {
	if body.len() > BODY_CEILING {
		return Err("is over the 262144-byte ceiling");
	}
	let text = std::str::from_utf8(body).map_err(|_| "is not UTF-8")?;
	if !text.trim_start_matches(JSON_WHITESPACE).starts_with(['{', '[']) {
		return Err("must be a JSON object or array");
	}
	serde_json::from_str::<serde::de::IgnoredAny>(text).map_err(|_| "is not JSON")?;
	Ok(())
}

fn raw(body: Option<String>) -> Result<Option<Box<RawValue>>, KoreanError> {
	body.map(|body| RawValue::from_string(body).map_err(|_| KoreanError::Unreadable)).transpose()
}

/// `key`'s latest revision of `kind`, removed or not, with its hash.
async fn latest(conn: &mut SqliteConnection, subject: &str, kind: Kind, key: &str) -> Result<Option<(Entry, Option<String>)>, KoreanError> {
	let kind = kind.as_str();
	let row = sqlx::query!(
		"SELECT revision, body, sha256, written_at FROM korean_entry WHERE subject_id = ?1 AND kind = ?2 AND entry_key = ?3 ORDER BY revision DESC LIMIT 1",
		subject,
		kind,
		key
	)
	.fetch_optional(&mut *conn)
	.await?;
	row
		.map(|row| {
			Ok((
				Entry {
					key: key.to_owned(),
					revision: row.revision,
					body: raw(row.body)?,
					written_at: row.written_at,
				},
				row.sha256,
			))
		})
		.transpose()
}

async fn insert(conn: &mut SqliteConnection, subject: &str, kind: Kind, key: &str, revision: i64, body: Option<&str>, now: &str) -> Result<(), KoreanError> {
	let kind = kind.as_str();
	let sha256 = body.map(|body| content_hash(body.as_bytes()));
	sqlx::query!(
		"INSERT INTO korean_entry (subject_id, kind, entry_key, revision, body, sha256, written_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
		subject,
		kind,
		key,
		revision,
		body,
		sha256,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok(())
}

/// Writes `body` as `key`'s next revision of `kind` for `subject`, as of
/// `now`, and answers what it did with the entry as it now stands.
///
/// What is kept is the body's bytes with the JSON whitespace around the
/// value trimmed, and nothing else changed.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the read that decides and
/// the write see one snapshot.
///
/// # Errors
/// [`KoreanError::Invalid`] naming every problem [`validate`] finds, decided
/// before anything is written, so a caller that goes on after one (a batch,
/// `file_host`'s `handlers::sync`) keeps nothing of it.
/// [`KoreanError::Storage`] for any `sqlx` failure.
pub async fn put(conn: &mut SqliteConnection, subject: &str, kind: &str, key: &str, body: &[u8], now: &str) -> Result<(Change, Entry), KoreanError> {
	let kind = validate(kind, key, Some(body)).map_err(KoreanError::Invalid)?;
	// `validate` held it to UTF-8. What is kept is the JSON value's own text:
	// whitespace around it is no part of it, and a `RawValue` (an answer, or a
	// batch item as it arrives) carries none, so the bytes kept, hashed and
	// answered are the same bytes whichever route wrote them.
	let text = std::str::from_utf8(body)
		.map_err(|_| KoreanError::Invalid(vec![("body", "is not UTF-8")]))?
		.trim_matches(JSON_WHITESPACE);
	let standing = latest(conn, subject, kind, key).await?;
	if let Some((entry, Some(sha256))) = &standing {
		if *sha256 == content_hash(text.as_bytes()) {
			return Ok((Change::Unchanged, entry.clone()));
		}
	}
	let revision = standing.as_ref().map_or(1, |(entry, _)| entry.revision + 1);
	insert(conn, subject, kind, key, revision, Some(text), now).await?;
	let entry = Entry {
		key: key.to_owned(),
		revision,
		body: raw(Some(text.to_owned()))?,
		written_at: now.to_owned(),
	};
	Ok((if standing.is_some() { Change::Revised } else { Change::Kept }, entry))
}

/// Removes `key` of `kind` for `subject`, as of `now`, as a revision with no
/// body, and answers what it did with that revision.
///
/// Nothing standing (never written, or already removed) writes nothing: the
/// answer is `None` for a key never written, or the removal that stands.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, as [`put`].
///
/// # Errors
/// As [`put`]'s.
pub async fn remove(conn: &mut SqliteConnection, subject: &str, kind: &str, key: &str, now: &str) -> Result<(Change, Option<Entry>), KoreanError> {
	let kind = validate(kind, key, None).map_err(KoreanError::Invalid)?;
	let Some((standing, _)) = latest(conn, subject, kind, key).await? else {
		return Ok((Change::Unchanged, None));
	};
	if standing.body.is_none() {
		return Ok((Change::Unchanged, Some(standing)));
	}
	let revision = standing.revision + 1;
	insert(conn, subject, kind, key, revision, None, now).await?;
	Ok((
		Change::Removed,
		Some(Entry {
			key: key.to_owned(),
			revision,
			body: None,
			written_at: now.to_owned(),
		}),
	))
}

/// Every entry of `kind` that stands for `subject` (its latest revision has
/// a body), in the order each key was first written, then by key: a word
/// set written again keeps its place, as on the phone.
///
/// Unbounded on purpose: it is one person's record, and the phone reads a
/// kind whole to study from it.
///
/// # Errors
/// [`KoreanError::Unreadable`] for a stored body that does not read back,
/// and [`KoreanError::Storage`] for any `sqlx` failure.
pub async fn standing(conn: &mut SqliteConnection, subject: &str, kind: Kind) -> Result<Vec<Entry>, KoreanError> {
	let kind = kind.as_str();
	let rows = sqlx::query!(
		r#"
		SELECT e.entry_key, e.revision, e.body AS "body!", e.written_at,
		       (SELECT f.written_at FROM korean_entry f
		        WHERE f.subject_id = e.subject_id AND f.kind = e.kind AND f.entry_key = e.entry_key AND f.revision = 1) AS "first: String"
		FROM korean_entry e
		WHERE e.subject_id = ?1 AND e.kind = ?2 AND e.body IS NOT NULL
		  AND e.revision = (SELECT MAX(r.revision) FROM korean_entry r
		                    WHERE r.subject_id = e.subject_id AND r.kind = e.kind AND r.entry_key = e.entry_key)
		ORDER BY 5, e.entry_key
		"#,
		subject,
		kind
	)
	.fetch_all(&mut *conn)
	.await?;
	rows
		.into_iter()
		.map(|row| {
			Ok(Entry {
				key: row.entry_key,
				revision: row.revision,
				body: raw(Some(row.body))?,
				written_at: row.written_at,
			})
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::{put, remove, standing, validate, Change, Kind, KoreanError};
	use serde_json::{json, Value};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	/// The migration before `20261026000200_create_korean_entry`.
	const PREVIOUS_MIGRATION: i64 = 20_261_026_000_100;
	const T0: &str = "2026-10-26T07:00:00.000Z";
	const T1: &str = "2026-10-26T12:00:00.000Z";
	const T2: &str = "2026-10-26T18:00:00.000Z";
	const A: &str = "subject-a";
	const B: &str = "subject-b";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	async fn write(pool: &SqlitePool, subject: &str, kind: &str, key: &str, body: &Value, now: &str) -> Result<Change, KoreanError> {
		put(&mut pool.acquire().await.unwrap(), subject, kind, key, body.to_string().as_bytes(), now)
			.await
			.map(|(change, _)| change)
	}

	async fn drop_key(pool: &SqlitePool, subject: &str, kind: &str, key: &str, now: &str) -> Change {
		remove(&mut pool.acquire().await.unwrap(), subject, kind, key, now).await.unwrap().0
	}

	/// Each standing entry of `kind` for `subject`: its key and body.
	async fn held(pool: &SqlitePool, subject: &str, kind: Kind) -> Vec<(String, Value)> {
		standing(&mut pool.acquire().await.unwrap(), subject, kind)
			.await
			.unwrap()
			.into_iter()
			.map(|entry| (entry.key, serde_json::from_str(entry.body.unwrap().get()).unwrap()))
			.collect()
	}

	async fn rows(pool: &SqlitePool) -> i64 {
		sqlx::query_scalar("SELECT COUNT(*) FROM korean_entry").fetch_one(pool).await.unwrap()
	}

	fn set(title: &str) -> Value {
		json!({ "deck": { "id": "cafe", "title": title, "words": [], "lines": [] }, "addedAt": T0, "openedAt": null })
	}

	#[tokio::test]
	async fn the_migration_round_trips_and_refuses_to_drop_a_record() {
		let pool = pool().await;
		let tables = || sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'korean_entry'");
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 1);
		MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 0, "down drops an empty table");
		MIGRATOR.run(&pool).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 1, "up recreates it");

		write(&pool, A, "set", "cafe", &set("Café"), T0).await.unwrap();
		assert!(MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.is_err(), "down refuses once the record holds a row");
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 1, "and leaves it as it was");
		assert_eq!(rows(&pool).await, 1);
	}

	/// The columns the migration describes and no others: a new one is a
	/// decision about what the server learns from the record.
	#[tokio::test]
	async fn the_table_holds_exactly_its_described_columns() {
		let pool = pool().await;
		let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('korean_entry') ORDER BY cid")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(columns, ["subject_id", "kind", "entry_key", "revision", "body", "sha256", "written_at"]);
	}

	#[tokio::test]
	async fn every_change_is_a_new_revision_and_every_old_one_stays() {
		let pool = pool().await;
		assert_eq!(write(&pool, A, "set", "cafe", &set("Café"), T0).await.unwrap(), Change::Kept);
		assert_eq!(
			write(&pool, A, "set", "cafe", &set("Café"), T1).await.unwrap(),
			Change::Unchanged,
			"the same bytes write nothing"
		);
		assert_eq!(write(&pool, A, "set", "cafe", &set("At the café"), T1).await.unwrap(), Change::Revised);
		assert_eq!(drop_key(&pool, A, "set", "cafe", T2).await, Change::Removed);
		assert_eq!(drop_key(&pool, A, "set", "cafe", T2).await, Change::Unchanged, "removing what is removed writes nothing");
		assert_eq!(drop_key(&pool, A, "set", "never", T2).await, Change::Unchanged, "nor does removing what was never kept");
		assert_eq!(held(&pool, A, Kind::Set).await, [], "a removed entry no longer stands");
		assert_eq!(write(&pool, A, "set", "cafe", &set("Café"), T2).await.unwrap(), Change::Revised, "and can be written again");
		assert_eq!(rows(&pool).await, 4, "every revision kept, the removal included");
		assert_eq!(held(&pool, A, Kind::Set).await, [(String::from("cafe"), set("Café"))]);

		let update = sqlx::query("UPDATE korean_entry SET body = '{}'").execute(&pool).await;
		assert!(update.unwrap_err().to_string().contains("append-only"), "no row is ever updated");
	}

	#[tokio::test]
	async fn the_answer_is_the_bytes_sent_verbatim() {
		let pool = pool().await;
		let sent = br#"{ "factor" : 1.25,"seen":3 }"#;
		let padded = b" \n{ \"factor\" : 1.25,\"seen\":3 }\r\n\t";
		let (change, entry) = put(&mut pool.acquire().await.unwrap(), A, "pace", "w-1", padded, T0).await.unwrap();
		assert_eq!(
			(change, entry.body.unwrap().get()),
			(Change::Kept, std::str::from_utf8(sent).unwrap()),
			"the value's own text, without the whitespace around it"
		);
		let (again, _) = put(&mut pool.acquire().await.unwrap(), A, "pace", "w-1", sent, T1).await.unwrap();
		assert_eq!(again, Change::Unchanged, "so the same value, padded or not, is the same bytes");
		let stood = standing(&mut pool.acquire().await.unwrap(), A, Kind::Pace).await.unwrap();
		assert_eq!(
			stood[0].body.as_ref().unwrap().get(),
			std::str::from_utf8(sent).unwrap(),
			"byte for byte, never re-serialised"
		);
	}

	/// Kinds and subjects stay apart, and each kind reads in the order its
	/// keys were first written, a key written again keeping its place.
	#[tokio::test]
	async fn entries_stand_apart_by_subject_and_kind_in_the_order_first_written() {
		let pool = pool().await;
		write(&pool, A, "set", "zebra", &json!({ "n": 1 }), T0).await.unwrap();
		write(&pool, A, "set", "apple", &json!({ "n": 2 }), T1).await.unwrap();
		write(&pool, A, "set", "zebra", &json!({ "n": 3 }), T2).await.unwrap();
		write(&pool, A, "pace", "zebra", &json!({ "factor": 1 }), T0).await.unwrap();
		write(&pool, B, "set", "mine", &json!({ "n": 4 }), T0).await.unwrap();
		assert_eq!(
			held(&pool, A, Kind::Set).await,
			[(String::from("zebra"), json!({ "n": 3 })), (String::from("apple"), json!({ "n": 2 }))]
		);
		assert_eq!(held(&pool, A, Kind::Pace).await, [(String::from("zebra"), json!({ "factor": 1 }))]);
		assert_eq!(held(&pool, B, Kind::Set).await, [(String::from("mine"), json!({ "n": 4 }))], "B sees only B's");
		assert_eq!(held(&pool, B, Kind::Pace).await, []);
	}

	#[tokio::test]
	async fn a_refused_write_names_each_field_and_writes_nothing() {
		let fields = |problems: Vec<(&'static str, &'static str)>| problems.into_iter().map(|(field, _)| field).collect::<Vec<_>>();
		assert_eq!(fields(validate("sets", ".hidden", Some(b"\"text\"")).unwrap_err()), ["kind", "key", "body"]);
		assert_eq!(fields(validate("set", "a.json", None).unwrap_err()), ["key"]);
		assert_eq!(fields(validate("tally", "k", Some(b"{ not json")).unwrap_err()), ["body"]);
		assert_eq!(fields(validate("tally", "k", Some(&vec![b'['; super::BODY_CEILING + 1])).unwrap_err()), ["body"]);
		assert_eq!(fields(validate("record", "it", Some(b"\xff")).unwrap_err()), ["body"]);
		assert_eq!(validate("feedback", "it", Some(b"[]")).unwrap(), Kind::Feedback);

		let pool = pool().await;
		assert!(matches!(write(&pool, A, "words", "w", &json!({}), T0).await, Err(KoreanError::Invalid(_))));
		assert!(matches!(
			remove(&mut pool.acquire().await.unwrap(), A, "set", "http-x", T0).await,
			Err(KoreanError::Invalid(_))
		));
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
			write(&pool, subject, "set", "cafe", &set("Café"), T0).await.unwrap();
		}
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		let left: Vec<String> = sqlx::query_scalar("SELECT subject_id FROM korean_entry").fetch_all(&pool).await.unwrap();
		assert_eq!(left, [B], "A's record went with A's account, B's stayed");
	}
}
