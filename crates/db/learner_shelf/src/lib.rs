//! The learner shelf (#387): content a learner generated themselves and
//! **chose** to keep on the server, to replay it on another device.
//!
//! One table, `learner_shelf`, serves every activity whose content a learner
//! can generate from a prompt — TOPIK lessons (#387) and `LeetType` rounds
//! (`paulgsc/some-ui#1598`) — keyed by [`Activity`] so the two never collide.
//! `20260929000200_create_learner_shelf.up.sql` records the design against
//! #387's five points and hard constraints; in short:
//!
//! - **Per subject, and only per subject.** Every query here takes the
//!   subject and filters on it; there is no function that reads across
//!   subjects, so there is no shared or public listing to leak.
//! - **Capped, and refused over the cap.** At most [`SHELF_CAP`] items per
//!   subject per activity. A new key past it is [`ShelfError::Full`]; nothing
//!   is ever evicted. Replacing an existing key is always allowed.
//! - **Content only, never parsed.** A body is stored byte for byte, after
//!   one check: it is a JSON object or array within [`SHELF_BODY_CEILING`]
//!   ([`validate`]). What the item *is* stays the client's to check (canon
//!   Def. 8.3: the server never parses a lesson).
//! - **Nowhere near the corpus.** Nothing here reads or writes `curriculum`,
//!   `curriculum_publication` or `leetype_round`, so a kept item is never
//!   served to anyone else and never announced by the study nudge. This crate
//!   borrows two pure functions from `curriculum_repo` ([`is_plain_key`],
//!   [`content_hash`]) so a key and a hash mean the same thing everywhere;
//!   it runs none of that crate's queries.
//! - **No background sync.** The only writers of `learner_shelf` are [`put`]
//!   and [`delete`], and their only callers are `file_host`'s
//!   `handlers::shelf` (`PUT` / `DELETE /api/v1/shelf/:activity/:key`) plus
//!   account deletion's generic sweep of `SUBJECT_SCOPED_TABLES`. A
//!   `git grep -n learner_shelf -- '*.rs'` shows no other writer: no waker
//!   pass, importer or binary touches it.

use curriculum_repo::content_hash;
/// The key rule, shared with the corpus: a shelf key is a URL path segment.
pub use curriculum_repo::is_plain_key;
use serde::Serialize;
use sqlx::SqliteConnection;

/// The most items one subject keeps for one activity. Matches the client's
/// `MAX_LOCAL_LESSONS`: a shelf, not a library.
pub const SHELF_CAP: i64 = 20;

/// The most bytes one kept body may hold. Lessons run to tens of KB, and a
/// round is held to the same 256 KiB as `leetype_round_repo::ROUND_BYTES_CEILING`.
pub const SHELF_BODY_CEILING: usize = 256 * 1024;

/// The activities whose learner-generated content a shelf holds. Closed, and
/// mirrored by the table's `CHECK`: another activity is a migration that says
/// why its content belongs here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
	Topik,
	Leetype,
}

impl Activity {
	/// The catalogue's activity id, and the `:activity` path segment.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Topik => "topik",
			Self::Leetype => "leetype",
		}
	}

	/// `None` for anything but a shelf activity's id.
	#[must_use]
	pub fn parse(raw: &str) -> Option<Self> {
		match raw {
			"topik" => Some(Self::Topik),
			"leetype" => Some(Self::Leetype),
			_ => None,
		}
	}
}

/// One kept item as a listing describes it: never its body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfEntry {
	pub key: String,
	/// Lowercase hex SHA-256 over the body's exact bytes.
	pub content_hash: String,
	/// ISO-8601 UTC, when these bytes were kept.
	pub saved_at: String,
}

/// What [`put`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
	/// A key this subject's shelf did not hold: a new row.
	Kept,
	/// The key was held with different bytes: body, hash and `saved_at`
	/// replaced.
	Replaced,
	/// The same bytes were already kept under this key: nothing written, and
	/// `saved_at` did not move.
	Unchanged,
}

impl Change {
	/// The wire name.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Kept => "kept",
			Self::Replaced => "replaced",
			Self::Unchanged => "unchanged",
		}
	}
}

/// What [`put`] did, and the item as it now stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Put {
	pub change: Change,
	pub entry: ShelfEntry,
}

/// Why [`is_plain_key`] refused a key, as [`validate`] reports it.
pub const KEY_PROBLEM: &str = "must be a plain key: URL-unreserved characters, not starting with `.` or `http`, no `.json` suffix";

/// `(field, why)`: one reason a write was refused.
pub type Problem = (&'static str, &'static str);

/// Why [`put`] wrote nothing.
#[derive(Debug)]
pub enum ShelfError {
	/// The key or body failed [`validate`]; every problem, by field.
	Invalid(Vec<Problem>),
	/// A new key, and this subject already keeps [`SHELF_CAP`] items for this
	/// activity. Nothing was evicted; the client decides what to delete.
	Full,
	Storage(sqlx::Error),
}

impl std::fmt::Display for ShelfError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Invalid(problems) => {
				f.write_str("not an item this shelf can keep:")?;
				for (field, why) in problems {
					write!(f, " {field} {why};")?;
				}
				Ok(())
			}
			Self::Full => write!(f, "the shelf already holds {SHELF_CAP} items for this activity"),
			Self::Storage(err) => write!(f, "database error: {err}"),
		}
	}
}

impl std::error::Error for ShelfError {}

impl From<sqlx::Error> for ShelfError {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

/// Whether `key` and `body` can be kept, every problem by field.
///
/// The key is a URL path segment, held to [`is_plain_key`]'s rule. The body
/// is checked for exactly what storing it verbatim needs: at most
/// [`SHELF_BODY_CEILING`] bytes, UTF-8 (the column is `TEXT`), and one JSON
/// object or array. It is read only as far as that; no value is built from it
/// and no field of it is looked at.
///
/// # Errors
/// Every problem found, as `(field, why)`.
pub fn validate(key: &str, body: &[u8]) -> Result<(), Vec<Problem>> {
	let mut problems = Vec::new();
	if !is_plain_key(key) {
		problems.push(("key", KEY_PROBLEM));
	}
	if let Err(why) = check_body(body) {
		problems.push(("body", why));
	}
	if problems.is_empty() {
		Ok(())
	} else {
		Err(problems)
	}
}

fn check_body(body: &[u8]) -> Result<(), &'static str> {
	if body.len() > SHELF_BODY_CEILING {
		return Err("is over the 262144-byte ceiling");
	}
	let text = std::str::from_utf8(body).map_err(|_| "is not UTF-8")?;
	if !text.trim_start_matches([' ', '\t', '\n', '\r']).starts_with(['{', '[']) {
		return Err("must be a JSON object or array");
	}
	serde_json::from_str::<serde::de::IgnoredAny>(text).map_err(|_| "is not JSON")?;
	Ok(())
}

/// Keep `body` under `key` on `subject`'s shelf for `activity`, as of `now`.
///
/// Idempotent by [`content_hash`]: the same bytes again are
/// [`Change::Unchanged`] and write nothing. Different bytes under a kept key
/// are [`Change::Replaced`], which the cap never refuses. A new key is
/// [`Change::Kept`] only while the shelf holds fewer than [`SHELF_CAP`] items
/// for that activity; otherwise [`ShelfError::Full`].
///
/// The cap is a condition of the `INSERT` itself, so no interleaving of
/// writes can exceed it. Run it in a `BEGIN IMMEDIATE` transaction all the
/// same, so the read that decides between the three outcomes and the write
/// see one snapshot.
///
/// # Errors
/// [`ShelfError::Invalid`] for a key or body [`validate`] refuses,
/// [`ShelfError::Full`] as above, and [`ShelfError::Storage`] for any `sqlx`
/// failure.
pub async fn put(conn: &mut SqliteConnection, subject: &str, activity: Activity, key: &str, body: &[u8], now: &str) -> Result<Put, ShelfError> {
	validate(key, body).map_err(ShelfError::Invalid)?;
	let hash = content_hash(body);
	// Exact: `validate` checked it is UTF-8.
	let text = String::from_utf8_lossy(body);
	let activity = activity.as_str();

	let stored = sqlx::query!(
		"SELECT content_hash, saved_at FROM learner_shelf WHERE subject_id = ?1 AND activity_id = ?2 AND key = ?3",
		subject,
		activity,
		key
	)
	.fetch_optional(&mut *conn)
	.await?;

	let change = match stored {
		Some(row) if row.content_hash == hash => {
			return Ok(Put {
				change: Change::Unchanged,
				entry: ShelfEntry {
					key: key.to_owned(),
					content_hash: hash,
					saved_at: row.saved_at,
				},
			});
		}
		Some(_) => {
			sqlx::query!(
				"UPDATE learner_shelf SET content_hash = ?4, saved_at = ?5, body = ?6 WHERE subject_id = ?1 AND activity_id = ?2 AND key = ?3",
				subject,
				activity,
				key,
				hash,
				now,
				text,
			)
			.execute(&mut *conn)
			.await?;
			Change::Replaced
		}
		None => {
			let inserted = sqlx::query!(
				r#"
				INSERT INTO learner_shelf (subject_id, activity_id, key, content_hash, saved_at, body)
				SELECT ?1, ?2, ?3, ?4, ?5, ?6
				WHERE (SELECT COUNT(*) FROM learner_shelf WHERE subject_id = ?1 AND activity_id = ?2) < ?7
				"#,
				subject,
				activity,
				key,
				hash,
				now,
				text,
				SHELF_CAP,
			)
			.execute(&mut *conn)
			.await?;
			if inserted.rows_affected() == 0 {
				return Err(ShelfError::Full);
			}
			Change::Kept
		}
	};
	Ok(Put {
		change,
		entry: ShelfEntry {
			key: key.to_owned(),
			content_hash: hash,
			saved_at: now.to_owned(),
		},
	})
}

/// `subject`'s shelf for `activity`, oldest kept first, then by key. Never a
/// body.
///
/// Unbounded in the query on purpose: [`put`] never lets a shelf past
/// [`SHELF_CAP`], and if the cap is ever lowered, a shelf already over it must
/// still list whole, or its owner could not see what to delete.
///
/// # Errors
/// Propagates any `sqlx` failure.
pub async fn list(conn: &mut SqliteConnection, subject: &str, activity: Activity) -> Result<Vec<ShelfEntry>, sqlx::Error> {
	let activity = activity.as_str();
	sqlx::query_as!(
		ShelfEntry,
		r#"SELECT key AS "key!", content_hash, saved_at FROM learner_shelf WHERE subject_id = ?1 AND activity_id = ?2 ORDER BY saved_at, key"#,
		subject,
		activity
	)
	.fetch_all(&mut *conn)
	.await
}

/// One kept body, verbatim, or `None` when `subject` keeps nothing under
/// `key` for `activity` — including when another subject does.
///
/// # Errors
/// Propagates any `sqlx` failure.
pub async fn body(conn: &mut SqliteConnection, subject: &str, activity: Activity, key: &str) -> Result<Option<String>, sqlx::Error> {
	let activity = activity.as_str();
	sqlx::query_scalar!(
		"SELECT body FROM learner_shelf WHERE subject_id = ?1 AND activity_id = ?2 AND key = ?3",
		subject,
		activity,
		key
	)
	.fetch_optional(&mut *conn)
	.await
}

/// Remove `key` from `subject`'s shelf for `activity`, and report whether it
/// was there. Only ever `subject`'s own row.
///
/// # Errors
/// Propagates any `sqlx` failure.
pub async fn delete(conn: &mut SqliteConnection, subject: &str, activity: Activity, key: &str) -> Result<bool, sqlx::Error> {
	let activity = activity.as_str();
	let deleted = sqlx::query!("DELETE FROM learner_shelf WHERE subject_id = ?1 AND activity_id = ?2 AND key = ?3", subject, activity, key)
		.execute(&mut *conn)
		.await?;
	Ok(deleted.rows_affected() == 1)
}

#[cfg(test)]
mod tests {
	use super::{body, delete, list, put, validate, Activity, Change, ShelfError, SHELF_BODY_CEILING, SHELF_CAP};
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const PREVIOUS_MIGRATION: i64 = 20_260_929_000_100;
	const T0: &str = "2026-09-29T00:00:00.000Z";
	const T1: &str = "2026-09-30T00:00:00.000Z";
	const A: &str = "subject-a";
	const B: &str = "subject-b";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	async fn keep(pool: &SqlitePool, subject: &str, activity: Activity, key: &str, bytes: &str, now: &str) -> Result<super::Put, ShelfError> {
		put(&mut pool.acquire().await.unwrap(), subject, activity, key, bytes.as_bytes(), now).await
	}

	async fn keys(pool: &SqlitePool, subject: &str, activity: Activity) -> Vec<String> {
		list(&mut pool.acquire().await.unwrap(), subject, activity)
			.await
			.unwrap()
			.into_iter()
			.map(|entry| entry.key)
			.collect()
	}

	async fn table_exists(pool: &SqlitePool) -> bool {
		sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!: i64" FROM sqlite_master WHERE type = 'table' AND name = 'learner_shelf'"#)
			.fetch_one(pool)
			.await
			.unwrap()
			== 1
	}

	#[tokio::test]
	async fn the_migration_round_trips() {
		let pool = pool().await;
		assert!(table_exists(&pool).await);
		MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
		assert!(!table_exists(&pool).await, "down drops it");
		MIGRATOR.run(&pool).await.unwrap();
		assert!(table_exists(&pool).await, "up recreates it");
	}

	/// #387 point 4, content only: the columns the migration describes and no
	/// others. A new column is a decision about what the server learns from a
	/// learner's shelf, not a detail.
	#[tokio::test]
	async fn the_table_holds_exactly_its_described_columns() {
		let pool = pool().await;
		let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('learner_shelf') ORDER BY cid")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(columns, ["subject_id", "activity_id", "key", "content_hash", "saved_at", "body"]);
	}

	/// Every query is a primary-key lookup or prefix range, never a scan.
	#[tokio::test]
	async fn every_query_reads_the_primary_key() {
		let pool = pool().await;
		for query in [
			"EXPLAIN QUERY PLAN SELECT key, content_hash, saved_at FROM learner_shelf WHERE subject_id = 'a' AND activity_id = 'topik' ORDER BY saved_at, key",
			"EXPLAIN QUERY PLAN SELECT body FROM learner_shelf WHERE subject_id = 'a' AND activity_id = 'topik' AND key = 'k'",
			"EXPLAIN QUERY PLAN SELECT COUNT(*) FROM learner_shelf WHERE subject_id = 'a' AND activity_id = 'topik'",
		] {
			let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(query).fetch_all(&pool).await.unwrap();
			assert!(
				plan.iter().any(|(_, _, _, detail)| detail.contains("sqlite_autoindex_learner_shelf_1")),
				"{query}: {plan:?}"
			);
			assert!(!plan.iter().any(|(_, _, _, detail)| detail.starts_with("SCAN learner_shelf")), "{query}: {plan:?}");
		}
	}

	/// The schema refuses an activity the shelf does not serve.
	#[tokio::test]
	async fn the_schema_refuses_another_activity() {
		let pool = pool().await;
		for (activity, ok) in [("topik", true), ("leetype", true), ("honeycomb", false)] {
			let inserted = sqlx::query("INSERT INTO learner_shelf (subject_id, activity_id, key, content_hash, saved_at, body) VALUES ('s', ?, 'k', 'h', 'now', '{}')")
				.bind(activity)
				.execute(&pool)
				.await;
			assert_eq!(inserted.is_ok(), ok, "{activity}");
		}
	}

	#[test]
	fn activities_parse_from_their_ids_and_nothing_else() {
		for activity in [Activity::Topik, Activity::Leetype] {
			assert_eq!(Activity::parse(activity.as_str()), Some(activity));
		}
		for raw in ["", "Topik", "honeycomb", "interview", "topik "] {
			assert_eq!(Activity::parse(raw), None, "{raw:?}");
		}
	}

	#[test]
	fn validation_names_each_field_and_checks_nothing_past_json() {
		assert_eq!(validate("lesson-1", b" {\"anything\": [1, {\"at\": \"all\"}]}\n"), Ok(()));
		assert_eq!(validate("lesson-1", b"[]"), Ok(()));
		for key in ["", "a/b", "a b", "lesson.json", ".hidden", "https:x", "a?b"] {
			assert_eq!(validate(key, b"{}").unwrap_err(), [("key", super::KEY_PROBLEM)], "{key:?}");
		}
		for (bytes, why) in [
			(&b"not json"[..], "must be a JSON object or array"),
			(b"\"a string\"", "must be a JSON object or array"),
			(b"42", "must be a JSON object or array"),
			(b"{\"open\": ", "is not JSON"),
			(b"{} {}", "is not JSON"),
			(b"{\"a\": \"\xff\"}", "is not UTF-8"),
		] {
			assert_eq!(validate("k", bytes).unwrap_err(), [("body", why)], "{bytes:?}");
		}
		let mut at_ceiling = vec![b' '; SHELF_BODY_CEILING - 2];
		at_ceiling.splice(0..0, *b"{}");
		assert_eq!(at_ceiling.len(), SHELF_BODY_CEILING);
		assert_eq!(validate("k", &at_ceiling), Ok(()));
		at_ceiling.push(b' ');
		assert_eq!(validate("k", &at_ceiling).unwrap_err(), [("body", "is over the 262144-byte ceiling")]);
		assert_eq!(validate("a/b", b"x").unwrap_err().len(), 2, "every problem, not the first");
	}

	/// Kept, then unchanged for the same bytes (nothing moves), then replaced
	/// for different ones; the body is stored and served byte for byte.
	#[tokio::test]
	async fn a_put_is_idempotent_by_content_hash() {
		let pool = pool().await;
		let bytes = "{ \"q\":  1 }\n";
		let kept = keep(&pool, A, Activity::Topik, "l1", bytes, T0).await.unwrap();
		assert_eq!(kept.change, Change::Kept);
		assert_eq!(kept.entry.content_hash, curriculum_repo::content_hash(bytes.as_bytes()));
		assert_eq!(kept.entry.saved_at, T0);

		let again = keep(&pool, A, Activity::Topik, "l1", bytes, T1).await.unwrap();
		assert_eq!(
			again,
			super::Put {
				change: Change::Unchanged,
				..kept.clone()
			},
			"same bytes: saved_at does not move"
		);

		let replaced = keep(&pool, A, Activity::Topik, "l1", "{\"q\":2}", T1).await.unwrap();
		assert_eq!((replaced.change, replaced.entry.saved_at.as_str()), (Change::Replaced, T1));
		assert_eq!(
			body(&mut pool.acquire().await.unwrap(), A, Activity::Topik, "l1").await.unwrap().as_deref(),
			Some("{\"q\":2}")
		);
		assert_eq!(list(&mut pool.acquire().await.unwrap(), A, Activity::Topik).await.unwrap(), [replaced.entry]);
	}

	/// #387 point 3: the 21st new key is refused and the first 20 are all
	/// still there. Replacing a kept key at the cap is allowed, the cap is per
	/// subject and per activity, and deleting one makes room again.
	#[tokio::test]
	async fn over_the_cap_a_new_key_is_refused_never_evicted() {
		let pool = pool().await;
		let cap = usize::try_from(SHELF_CAP).unwrap();
		let expected: Vec<String> = (0..cap).map(|i| String::from(if i < 10 { "l0" } else { "l" }) + &i.to_string()).collect();
		for key in &expected {
			assert_eq!(keep(&pool, A, Activity::Topik, key, "{}", T0).await.unwrap().change, Change::Kept);
		}
		assert!(matches!(keep(&pool, A, Activity::Topik, "one-more", "{}", T1).await, Err(ShelfError::Full)));
		assert_eq!(keys(&pool, A, Activity::Topik).await, expected, "nothing evicted, nothing added");

		assert_eq!(keep(&pool, A, Activity::Topik, "l00", "[1]", T1).await.unwrap().change, Change::Replaced);
		assert_eq!(keep(&pool, A, Activity::Leetype, "r1", "{}", T0).await.unwrap().change, Change::Kept, "per activity");
		assert_eq!(keep(&pool, B, Activity::Topik, "l1", "{}", T0).await.unwrap().change, Change::Kept, "per subject");

		assert!(delete(&mut pool.acquire().await.unwrap(), A, Activity::Topik, "l05").await.unwrap());
		assert_eq!(keep(&pool, A, Activity::Topik, "one-more", "{}", T1).await.unwrap().change, Change::Kept);
	}

	/// A refused key or body writes nothing.
	#[tokio::test]
	async fn an_invalid_put_writes_nothing() {
		let pool = pool().await;
		assert!(matches!(keep(&pool, A, Activity::Topik, "a/b", "{}", T0).await, Err(ShelfError::Invalid(_))));
		assert!(matches!(keep(&pool, A, Activity::Topik, "k", "nope", T0).await, Err(ShelfError::Invalid(_))));
		assert!(keys(&pool, A, Activity::Topik).await.is_empty());
	}

	/// Listed oldest first, then by key; never another subject's or another
	/// activity's; and a read, a delete or a replace for one subject never
	/// reaches another's row under the same key.
	#[tokio::test]
	async fn every_read_and_write_is_the_subjects_own() {
		let pool = pool().await;
		keep(&pool, A, Activity::Topik, "b", "{}", T1).await.unwrap();
		keep(&pool, A, Activity::Topik, "c", "{}", T0).await.unwrap();
		keep(&pool, A, Activity::Topik, "a", "{}", T1).await.unwrap();
		keep(&pool, A, Activity::Leetype, "z", "{}", T0).await.unwrap();
		assert_eq!(keys(&pool, A, Activity::Topik).await, ["c", "a", "b"]);
		assert_eq!(keys(&pool, A, Activity::Leetype).await, ["z"]);
		assert!(keys(&pool, B, Activity::Topik).await.is_empty());

		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(body(&mut conn, B, Activity::Topik, "a").await.unwrap(), None);
		assert_eq!(body(&mut conn, A, Activity::Leetype, "a").await.unwrap(), None, "keyed by activity too");
		assert!(!delete(&mut conn, B, Activity::Topik, "a").await.unwrap());
		drop(conn);
		assert_eq!(keep(&pool, B, Activity::Topik, "a", "[2]", T1).await.unwrap().change, Change::Kept, "B's own row");
		assert_eq!(body(&mut pool.acquire().await.unwrap(), A, Activity::Topik, "a").await.unwrap().as_deref(), Some("{}"));
		assert!(delete(&mut pool.acquire().await.unwrap(), A, Activity::Topik, "a").await.unwrap());
		assert!(!delete(&mut pool.acquire().await.unwrap(), A, Activity::Topik, "a").await.unwrap(), "idempotent");
		assert_eq!(keys(&pool, B, Activity::Topik).await, ["a"]);
	}
}
