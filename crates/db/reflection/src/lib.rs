//! The owner's written reflections (paulgsc/some-ui#2178, HF3): each typed
//! line the phone keeps about a study session, sent home by its drain.
//!
//! `20261101000100_create_written_reflection.up.sql` records the design; in
//! short:
//!
//! - **Append-only.** [`put_reflection`] stores a reflection under its id, or
//!   nothing when that id already holds the same one; a different reflection
//!   under a held id is refused ([`ReflectionError::Rewritten`]), never
//!   stored. No row is ever updated (a trigger aborts an `UPDATE`), and
//!   nothing here deletes. Account deletion is the one delete: its generic
//!   sweep of `SUBJECT_SCOPED_TABLES`, and the migration's trigger on
//!   `account`.
//! - **Written only.** A reflection here is a typed line. A spoken one goes
//!   home as a voice note (`voice_note`, docs/identity.md invariant 23), and
//!   the one-tap answers stay on the phone: either is refused by name.
//! - **The phone's rules, held here too.** A write passes [`parse`].
//! - **Per subject, and only per subject.** Every query takes the subject and
//!   filters on it; nothing reads across subjects.
//! - **No background writer.** The one caller is `file_host`'s
//!   `handlers::routine` (`PUT /api/v1/routine/reflections/:id`). A
//!   `git grep -n written_reflection -- '*.rs'` shows no other writer.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;

/// A refused field and why, as `file_host` answers it in a `422`.
pub type Problem = (&'static str, &'static str);

/// The largest body a write takes: far above any typed line.
pub const BODY_CEILING: usize = 64 * 1024;

/// The most characters one written reflection holds. The phone's field
/// takes no more (paulgsc/some-ui `session-reflection/policy.ts`,
/// `REFLECTION_TEXT_MAX`).
pub const MAX_TEXT_CHARS: usize = 10_000;

/// The most characters of the phone's id for a reflection.
const MAX_ID_CHARS: usize = 64;

/// The most characters of the phone's id for a session.
const MAX_SESSION_CHARS: usize = 200;

/// What a put did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
	/// Home did not hold it: it is stored now.
	Added,
	/// Home already holds this very reflection: nothing written.
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
pub enum ReflectionError {
	/// The write breaks a rule; every problem, by field.
	Invalid(Vec<Problem>),
	/// The id already holds a different reflection, which is never replaced.
	Rewritten,
	Storage(sqlx::Error),
}

impl std::fmt::Display for ReflectionError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Invalid(problems) => {
				f.write_str("not a written reflection home takes:")?;
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

impl std::error::Error for ReflectionError {}

impl From<sqlx::Error> for ReflectionError {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

/// What [`ReflectionError::Rewritten`] says.
pub const REWRITTEN: &str = "this reflection id already holds a different reflection: the record is append-only";

/// A written reflection's body, as the phone keeps it: `{ kind, text }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Written {
	/// Always `text`.
	pub kind: &'static str,
	/// The typed line, as typed.
	pub text: String,
}

/// A written reflection, in the phone's own shape
/// (paulgsc/some-ui `session-reflection/policy.ts`, `Reflection`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reflection {
	/// The phone's id for it: what makes a retried send store nothing.
	pub id: String,
	/// The phone's id for the session it is about.
	pub session_id: String,
	/// When it was written, as ISO-8601 UTC to the millisecond.
	pub created_at: String,
	/// How far into the session, in milliseconds, or `None` when written
	/// after it ended.
	pub at_ms: Option<i64>,
	pub body: Written,
}

/// What the phone sends, before it is checked.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Wire {
	id: String,
	session_id: String,
	created_at: String,
	#[serde(default)]
	at_ms: Option<i64>,
	body: serde_json::Map<String, serde_json::Value>,
}

/// The phone's own ids (`crypto.randomUUID()` today): URL-safe, short.
fn is_id(text: &str) -> bool {
	!text.is_empty() && text.len() <= MAX_ID_CHARS && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// A session's id, as a voice note's `ref` names it.
fn is_session(text: &str) -> bool {
	!text.is_empty() && text.len() <= MAX_SESSION_CHARS && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

/// `at` as ISO-8601 UTC to the millisecond: what is stored and served.
///
/// `None` when it is not an RFC 3339 instant in UTC (`Z`, as the phone's
/// `toISOString()` writes).
#[must_use]
pub fn instant(at: &str) -> Option<String> {
	if !at.ends_with('Z') {
		return None;
	}
	DateTime::parse_from_rfc3339(at)
		.ok()
		.map(|at| at.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Millis, true))
}

/// The written reflection in `body`, checked: the one `id` names.
///
/// # Errors
/// Every problem found, as `(field, why)`.
pub fn parse(id: &str, body: &[u8]) -> Result<Reflection, Vec<Problem>> {
	if body.len() > BODY_CEILING {
		return Err(vec![("body", "is over the size ceiling")]);
	}
	// Straight from the bytes, so a key written twice is refused rather than
	// read as its last copy.
	let wire: Wire = serde_json::from_slice(body).map_err(|_| vec![("body", "is not the shape a reflection has")])?;
	let mut problems = Vec::new();
	if !is_id(id) {
		problems.push(("id", "must be the phone's id: 1 to 64 letters, digits, `-` or `_`"));
	} else if wire.id != id {
		problems.push(("id", "must be the id the path names"));
	}
	if !is_session(&wire.session_id) {
		problems.push(("sessionId", "must be the phone's id for the session: 1 to 200 letters, digits, '-', '_', '.' or ':'"));
	}
	let created_at = instant(&wire.created_at);
	if created_at.is_none() {
		problems.push(("createdAt", "must be an RFC 3339 instant in UTC, ending `Z`"));
	}
	if wire.at_ms.is_some_and(|at_ms| at_ms < 0) {
		problems.push(("atMs", "must be how far into the session, in milliseconds, or null"));
	}
	let text = written(&wire.body, &mut problems);
	match (created_at, text) {
		(Some(created_at), Some(text)) if problems.is_empty() => Ok(Reflection {
			id: wire.id,
			session_id: wire.session_id,
			created_at,
			at_ms: wire.at_ms,
			body: Written { kind: "text", text },
		}),
		_ => Err(problems),
	}
}

/// The typed line in `body`, or `None` with why in `problems`.
fn written(body: &serde_json::Map<String, serde_json::Value>, problems: &mut Vec<Problem>) -> Option<String> {
	match body.get("kind").and_then(serde_json::Value::as_str) {
		Some("text") => {}
		Some("voice") => {
			problems.push(("body.kind", "a spoken reflection goes home as a voice note, not here"));
			return None;
		}
		Some("answers") => {
			problems.push(("body.kind", "the one-tap answers stay on the phone"));
			return None;
		}
		_ => {
			problems.push(("body.kind", "must be text"));
			return None;
		}
	}
	if body.keys().any(|key| key != "kind" && key != "text") {
		problems.push(("body", "holds only kind and text"));
		return None;
	}
	match body.get("text").and_then(serde_json::Value::as_str) {
		Some(text) if !text.trim().is_empty() && text.chars().count() <= MAX_TEXT_CHARS => Some(text.to_owned()),
		_ => {
			problems.push(("body.text", "must be the typed line: 1 to 10,000 characters, not blank"));
			None
		}
	}
}

/// Stores `body` as reflection `id` for `subject`, as of `now`, and answers
/// what it did with the reflection as it now stands.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the read that decides and
/// the write see one snapshot.
///
/// # Errors
/// [`ReflectionError::Invalid`] naming every problem [`parse`] finds,
/// [`ReflectionError::Rewritten`] when `id` already holds a different
/// reflection, and [`ReflectionError::Storage`] for any `sqlx` failure. A
/// refusal is decided before anything is written.
pub async fn put_reflection(conn: &mut SqliteConnection, subject: &str, id: &str, body: &[u8], now: &str) -> Result<(Change, Reflection), ReflectionError> {
	let reflection = parse(id, body).map_err(ReflectionError::Invalid)?;
	let held = sqlx::query!(
		"SELECT session_id, at, at_ms, text FROM written_reflection WHERE subject_id = ?1 AND reflection_id = ?2",
		subject,
		id
	)
	.fetch_optional(&mut *conn)
	.await?;
	if let Some(held) = held {
		let held = (held.session_id.as_str(), held.at.as_str(), held.at_ms, held.text.as_str());
		let sent = (
			reflection.session_id.as_str(),
			reflection.created_at.as_str(),
			reflection.at_ms,
			reflection.body.text.as_str(),
		);
		let same = held == sent;
		return if same { Ok((Change::Unchanged, reflection)) } else { Err(ReflectionError::Rewritten) };
	}
	sqlx::query!(
		"INSERT INTO written_reflection (subject_id, reflection_id, session_id, at, at_ms, text, written_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
		subject,
		id,
		reflection.session_id,
		reflection.created_at,
		reflection.at_ms,
		reflection.body.text,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok((Change::Added, reflection))
}

#[cfg(test)]
mod tests {
	use super::{parse, put_reflection, Change, ReflectionError, MAX_TEXT_CHARS};
	use serde_json::{json, Value};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const PREVIOUS_MIGRATION: i64 = 20_261_031_000_400;
	const NOW: &str = "2026-11-01T09:00:00.000Z";
	const A: &str = "subject-a";
	const B: &str = "subject-b";
	const S: &str = "4f0c2a8e-7b1d-4c3e-9a55-0d1e2f3a4b5c";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn typed(id: &str, text: &str) -> Value {
		json!({ "id": id, "sessionId": S, "createdAt": "2026-11-01T08:15:00.000Z", "atMs": 61_000, "body": { "kind": "text", "text": text } })
	}

	async fn put(pool: &SqlitePool, subject: &str, value: &Value) -> Result<Change, ReflectionError> {
		let id = value["id"].as_str().unwrap();
		put_reflection(&mut pool.acquire().await.unwrap(), subject, id, value.to_string().as_bytes(), NOW)
			.await
			.map(|(change, _)| change)
	}

	async fn rows(pool: &SqlitePool) -> i64 {
		sqlx::query_scalar("SELECT COUNT(*) FROM written_reflection").fetch_one(pool).await.unwrap()
	}

	fn fields(id: &str, value: &Value) -> Vec<&'static str> {
		let mut fields: Vec<&'static str> = parse(id, value.to_string().as_bytes()).unwrap_err().iter().map(|(field, _)| *field).collect();
		fields.sort_unstable();
		fields
	}

	/// Down refuses while the table holds a row, and drops it once empty.
	#[tokio::test]
	async fn the_migration_round_trips_and_refuses_to_drop_a_record() {
		let pool = pool().await;
		let tables = || sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'written_reflection'");
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 1);
		MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 0, "down drops an empty table");
		MIGRATOR.run(&pool).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 1, "up recreates it");
		put(&pool, A, &typed("r1", "slow")).await.unwrap();
		assert!(MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.is_err(), "down refuses a table holding a reflection");
		assert_eq!(rows(&pool).await, 1, "and leaves it as it was");
	}

	/// The columns the migration describes and no others: a new one is a
	/// decision about what home learns from a reflection.
	#[tokio::test]
	async fn the_table_holds_exactly_its_described_columns() {
		let pool = pool().await;
		let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('written_reflection') ORDER BY cid")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(columns, ["subject_id", "reflection_id", "session_id", "at", "at_ms", "text", "written_at"]);
	}

	#[tokio::test]
	async fn a_reflection_is_stored_once_and_never_replaced() {
		let pool = pool().await;
		let first = typed("r1", "The second scene felt slow.");
		assert_eq!(put(&pool, A, &first).await.unwrap(), Change::Added);
		assert_eq!(put(&pool, A, &first).await.unwrap(), Change::Unchanged, "the same reflection writes nothing");
		for changed in [
			typed("r1", "Something else."),
			json!({ "id": "r1", "sessionId": "another", "createdAt": "2026-11-01T08:15:00.000Z", "atMs": 61_000, "body": { "kind": "text", "text": "The second scene felt slow." } }),
			json!({ "id": "r1", "sessionId": S, "createdAt": "2026-11-01T08:15:00.000Z", "atMs": null, "body": { "kind": "text", "text": "The second scene felt slow." } }),
		] {
			assert!(
				matches!(put(&pool, A, &changed).await, Err(ReflectionError::Rewritten)),
				"a different reflection under a held id: {changed}"
			);
		}
		assert_eq!(rows(&pool).await, 1);
		assert_eq!(put(&pool, B, &typed("r1", "Something else.")).await.unwrap(), Change::Added, "B's ids are B's");

		let update = sqlx::query("UPDATE written_reflection SET text = 'x'").execute(&pool).await;
		assert!(update.unwrap_err().to_string().contains("append-only"), "no row is ever updated");
	}

	#[tokio::test]
	async fn the_answer_is_the_reflection_as_the_phone_keeps_it() {
		let pool = pool().await;
		let sent = typed("r1", "  more level two, please ");
		let (_, held) = put_reflection(&mut pool.acquire().await.unwrap(), A, "r1", sent.to_string().as_bytes(), NOW).await.unwrap();
		assert_eq!(serde_json::to_value(&held).unwrap(), sent, "byte for byte what the phone keeps, spaces and all");
		let after = json!({ "id": "r2", "sessionId": S, "createdAt": "2026-11-01T08:40:00.000Z", "atMs": null, "body": { "kind": "text", "text": "after" } });
		let (_, held) = put_reflection(&mut pool.acquire().await.unwrap(), A, "r2", after.to_string().as_bytes(), NOW)
			.await
			.unwrap();
		assert_eq!(serde_json::to_value(&held).unwrap(), after, "one written after the session says so");
	}

	#[test]
	fn a_spoken_reflection_or_the_answers_are_refused_by_name() {
		let voice = json!({ "id": "r1", "sessionId": S, "createdAt": "2026-11-01T08:15:00.000Z", "atMs": 0, "body": { "kind": "voice", "durationMs": 4000, "mimeType": "audio/webm", "bytes": 512 } });
		let answers =
			json!({ "id": "r1", "sessionId": S, "createdAt": "2026-11-01T08:15:00.000Z", "atMs": null, "body": { "kind": "answers", "answers": { "difficulty": "right" } } });
		for value in [voice, answers] {
			assert_eq!(fields("r1", &value), ["body.kind"], "{value}");
		}
	}

	#[test]
	fn refusals_name_each_field() {
		let mut bad = typed("r1", " ");
		bad["sessionId"] = json!("a/b");
		bad["createdAt"] = json!("2026-11-01T08:15:00+09:00");
		bad["atMs"] = json!(-1);
		assert_eq!(fields("r1", &bad), ["atMs", "body.text", "createdAt", "sessionId"]);
		assert_eq!(fields("r2", &typed("r1", "x")), ["id"], "the id the path names");
		assert_eq!(fields("r/1", &typed("r/1", "x")), ["id"]);
		let mut extra = typed("r1", "x");
		extra["body"]["audio"] = json!("T2dnUw");
		assert_eq!(fields("r1", &extra), ["body"]);
		let mut unknown = typed("r1", "x");
		unknown["durationMs"] = json!(1);
		assert_eq!(fields("r1", &unknown), ["body"], "a key the shape lacks");
		assert_eq!(fields("r1", &typed("r1", &"가".repeat(MAX_TEXT_CHARS + 1))), ["body.text"]);
		assert!(
			parse("r1", typed("r1", &"가".repeat(MAX_TEXT_CHARS)).to_string().as_bytes()).is_ok(),
			"the ceiling counts characters, not bytes"
		);
		let twice = br#"{"id":"r1","id":"r1","sessionId":"s","createdAt":"2026-11-01T08:15:00.000Z","atMs":null,"body":{"kind":"text","text":"x"}}"#;
		assert!(parse("r1", twice).is_err(), "a key written twice");
	}

	#[tokio::test]
	async fn a_refused_reflection_writes_nothing() {
		let pool = pool().await;
		assert!(matches!(put(&pool, A, &typed("r1", "")).await, Err(ReflectionError::Invalid(_))));
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
			put(&pool, subject, &typed("r1", "slow")).await.unwrap();
		}
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		let left: Vec<String> = sqlx::query_scalar("SELECT subject_id FROM written_reflection").fetch_all(&pool).await.unwrap();
		assert_eq!(left, [B], "A's reflections went with A's account, B's stayed");
	}
}
