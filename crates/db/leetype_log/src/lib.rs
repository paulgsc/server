//! The owner's `leetype` session log (paulgsc/some-ui
//! `packages/ui/leetype/src/lib/leetype/session-log`): every phone session of
//! incident rounds, as the append-only list of what happened in it.
//!
//! `20261020000100_create_leetype_session_event.up.sql` records the design;
//! in short:
//!
//! - **Append-only.** [`put_event`] stores an event under its session and id,
//!   or nothing when that id already holds the same event; a different body
//!   under a held id is refused ([`LogError::Rewritten`]), never stored. No
//!   row is ever updated (a trigger aborts an `UPDATE`), and nothing here
//!   deletes. Account deletion is the one delete: its generic sweep of
//!   `SUBJECT_SCOPED_TABLES`, and the migration's trigger on `account`.
//! - **The phone's rules, held here too.** A write passes [`parse_event`]:
//!   the event's shape and each field's, as the phone records them.
//! - **No soundbite.** What was said or typed in a spoken answer, and what it
//!   was marked against ([`SOUNDBITES`]), goes to the owner's Drive; an event
//!   carrying any of it is refused.
//! - **Per subject, and only per subject.** Every query takes the subject and
//!   filters on it; nothing reads across subjects.
//! - **No background writer.** The one caller is `file_host`'s
//!   `handlers::leetype_log` (`PUT /api/v1/leetype/sessions/:session/events/:id`),
//!   which the phone sends from its tapped "Send to home". A
//!   `git grep -n leetype_session_event -- '*.rs'` shows no other writer.

mod model;

pub use model::{instant, parse_event, Answer, Event, Form, How, Offered, Problem, ReadingAction, SpokenAnswer, EVENT_BODY_CEILING, SOUNDBITES};
use serde::Serialize;
use sqlx::SqliteConnection;

/// The most sessions [`sessions`] lists: the newest.
pub const SESSIONS_LISTED: i64 = 1_000;

/// What a put did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
	/// The session did not hold it: it is stored now.
	Added,
	/// The session already holds this very event: nothing written.
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
pub enum LogError {
	/// The write breaks a rule; every problem, by field.
	Invalid(Vec<Problem>),
	/// The id already holds a different event, which is never replaced.
	Rewritten,
	/// A stored body this crate wrote does not read back.
	Unreadable,
	Storage(sqlx::Error),
}

impl std::fmt::Display for LogError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Invalid(problems) => {
				f.write_str("not a write the session log takes:")?;
				for (field, why) in problems {
					write!(f, " {field} {why};")?;
				}
				Ok(())
			}
			Self::Rewritten => f.write_str(REWRITTEN),
			Self::Unreadable => f.write_str(UNREADABLE),
			Self::Storage(err) => write!(f, "database error: {err}"),
		}
	}
}

impl std::error::Error for LogError {}

impl From<sqlx::Error> for LogError {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

/// What [`LogError::Rewritten`] says.
pub const REWRITTEN: &str = "this event id already holds a different event: the session log is append-only";

/// What [`LogError::Unreadable`] says.
pub const UNREADABLE: &str = "a stored session-log event does not read back";

/// One session's events as the record holds them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionEvents {
	pub session: String,
	/// In the order the phone recorded them: by `at`, then the order they
	/// reached home, which is the phone's log order (it sends in order).
	pub events: Vec<Event>,
}

/// One session, summarised.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
	pub session: String,
	/// The session file its `started` event names, or `None` while home holds
	/// no `started` event for it.
	pub file: Option<String>,
	/// Its first and last event's `at`, as ISO-8601 UTC to the millisecond.
	pub first_at: String,
	pub last_at: String,
	/// How many events home holds for it.
	pub events: i64,
}

/// The sessions home holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Sessions {
	/// Newest first, by last event, at most [`SESSIONS_LISTED`].
	pub sessions: Vec<Session>,
}

fn read_body(body: &str) -> Result<Event, LogError> {
	serde_json::from_str(body).map_err(|_| LogError::Unreadable)
}

/// Stores `body` as event `id` of `session` for `subject`, as of `now`, and
/// answers what it did with the event as it now stands.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the read that decides and
/// the write see one snapshot.
///
/// # Errors
/// [`LogError::Invalid`] naming every problem [`parse_event`] finds,
/// [`LogError::Rewritten`] when `id` already holds a different event in
/// `session`, and [`LogError::Storage`] for any `sqlx` failure. A refusal is
/// decided before anything is written.
pub async fn put_event(conn: &mut SqliteConnection, subject: &str, session: &str, id: &str, body: &[u8], now: &str) -> Result<(Change, Event), LogError> {
	let event = parse_event(session, id, body).map_err(LogError::Invalid)?;
	let standing = sqlx::query_scalar!(
		"SELECT body FROM leetype_session_event WHERE subject_id = ?1 AND session_id = ?2 AND event_id = ?3",
		subject,
		session,
		id
	)
	.fetch_optional(&mut *conn)
	.await?;
	if let Some(standing) = standing {
		return if read_body(&standing)? == event {
			Ok((Change::Unchanged, event))
		} else {
			Err(LogError::Rewritten)
		};
	}
	let at = instant(event.at()).ok_or(LogError::Unreadable)?;
	let kind = event.kind();
	let text = serde_json::to_value(&event).map(|json| json.to_string()).map_err(|_| LogError::Unreadable)?;
	sqlx::query!(
		"INSERT INTO leetype_session_event (subject_id, session_id, event_id, at, type, body, written_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
		subject,
		session,
		id,
		at,
		kind,
		text,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok((Change::Added, event))
}

/// `session`'s events for `subject`, as they stand: none for a session home
/// has never seen.
///
/// Unbounded on purpose: one session is a few dozen events, and the phone
/// reads them back whole to confirm what home holds before it lets them go.
///
/// # Errors
/// [`LogError::Unreadable`] for a stored body that does not read back, and
/// [`LogError::Storage`] for any `sqlx` failure.
pub async fn session_events(conn: &mut SqliteConnection, subject: &str, session: &str) -> Result<SessionEvents, LogError> {
	let bodies = sqlx::query_scalar!(
		"SELECT body FROM leetype_session_event WHERE subject_id = ?1 AND session_id = ?2 ORDER BY at, rowid",
		subject,
		session
	)
	.fetch_all(&mut *conn)
	.await?;
	let events = bodies.iter().map(|body| read_body(body)).collect::<Result<Vec<Event>, LogError>>()?;
	Ok(SessionEvents {
		session: session.to_owned(),
		events,
	})
}

/// The newest [`SESSIONS_LISTED`] of `subject`'s sessions, newest first.
///
/// # Errors
/// [`LogError::Storage`] for any `sqlx` failure.
pub async fn sessions(conn: &mut SqliteConnection, subject: &str) -> Result<Sessions, LogError> {
	let rows = sqlx::query!(
		r#"SELECT
			session_id AS "session!: String",
			MIN(at) AS "first_at!: String",
			MAX(at) AS "last_at!: String",
			COUNT(*) AS "events!: i64",
			(
				SELECT json_extract(started.body, '$.file')
				FROM leetype_session_event AS started
				WHERE started.subject_id = ?1 AND started.session_id = held.session_id AND started.type = 'started'
				ORDER BY started.at, started.rowid
				LIMIT 1
			) AS "file?: String"
		FROM leetype_session_event AS held
		WHERE subject_id = ?1
		GROUP BY session_id
		ORDER BY MAX(at) DESC, session_id
		LIMIT ?2"#,
		subject,
		SESSIONS_LISTED
	)
	.fetch_all(&mut *conn)
	.await?;
	let sessions = rows
		.into_iter()
		.map(|row| Session {
			session: row.session,
			file: row.file,
			first_at: row.first_at,
			last_at: row.last_at,
			events: row.events,
		})
		.collect();
	Ok(Sessions { sessions })
}

#[cfg(test)]
mod tests {
	use super::{put_event, session_events, sessions, Change, LogError, SESSIONS_LISTED};
	use serde_json::{json, Value};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const PREVIOUS_MIGRATION: i64 = 20_261_019_000_100;
	const NOW: &str = "2026-10-20T23:00:00.000Z";
	const A: &str = "subject-a";
	const B: &str = "subject-b";
	const S1: &str = "s1";
	const S2: &str = "s2";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn started(session: &str, id: &str, at: &str) -> Value {
		json!({ "id": id, "at": at, "session": session, "type": "started", "file": "raft-1.json", "form": "full", "declaredMinutes": 25 })
	}

	fn parked(session: &str, id: &str, at: &str) -> Value {
		json!({ "id": id, "at": at, "session": session, "type": "parked", "file": "raft-1.json", "screen": "r1" })
	}

	async fn put(pool: &SqlitePool, subject: &str, value: &Value) -> Result<Change, LogError> {
		let (session, id) = (value["session"].as_str().unwrap(), value["id"].as_str().unwrap());
		put_event(&mut pool.acquire().await.unwrap(), subject, session, id, value.to_string().as_bytes(), NOW)
			.await
			.map(|(change, _)| change)
	}

	async fn rows(pool: &SqlitePool) -> i64 {
		sqlx::query_scalar("SELECT COUNT(*) FROM leetype_session_event").fetch_one(pool).await.unwrap()
	}

	async fn ids(pool: &SqlitePool, subject: &str, session: &str) -> Vec<String> {
		let held = session_events(&mut pool.acquire().await.unwrap(), subject, session).await.unwrap();
		assert_eq!(held.session, session);
		held.events.iter().map(|event| event.id().to_owned()).collect()
	}

	#[tokio::test]
	async fn the_migration_round_trips() {
		let pool = pool().await;
		let tables = || sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'leetype_session_event'");
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 1);
		MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 0, "down drops it");
		MIGRATOR.run(&pool).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 1, "up recreates it");
	}

	/// The columns the migration describes and no others: a new one is a
	/// decision about what the server learns from the record.
	#[tokio::test]
	async fn the_table_holds_exactly_its_described_columns() {
		let pool = pool().await;
		let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('leetype_session_event') ORDER BY cid")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(columns, ["subject_id", "session_id", "event_id", "at", "type", "body", "written_at"]);
	}

	/// The three lookups by one session go by the subject and the session: a
	/// put's by the primary key, a session's read-back and the list's
	/// `started` lookup by `leetype_session_event_order`, in `at` order with
	/// no sort. None scans, as an index on `(subject_id, at)` alone did once a
	/// subject held thousands of sessions. The queries mirror `put_event`,
	/// `session_events` and `sessions`, with literals for their binds.
	#[tokio::test]
	async fn every_read_goes_by_the_subject_and_the_session() {
		let pool = pool().await;
		for (query, index) in [
			(
				"EXPLAIN QUERY PLAN SELECT body FROM leetype_session_event WHERE subject_id = 'a' AND session_id = 's' AND event_id = 'e'",
				"sqlite_autoindex_leetype_session_event_1 (subject_id=? AND session_id=? AND event_id=?)",
			),
			(
				"EXPLAIN QUERY PLAN SELECT body FROM leetype_session_event WHERE subject_id = 'a' AND session_id = 's' ORDER BY at, rowid",
				"leetype_session_event_order (subject_id=? AND session_id=?)",
			),
			(
				"EXPLAIN QUERY PLAN SELECT json_extract(started.body, '$.file') FROM leetype_session_event AS started
				 WHERE started.subject_id = 'a' AND started.session_id = 's' AND started.type = 'started' ORDER BY started.at, started.rowid LIMIT 1",
				"leetype_session_event_order (subject_id=? AND session_id=?)",
			),
		] {
			let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(query).fetch_all(&pool).await.unwrap();
			assert!(plan.iter().any(|(_, _, _, detail)| detail.contains(index)), "{query}: {plan:?}");
			assert!(!plan.iter().any(|(_, _, _, detail)| detail.starts_with("SCAN")), "{query}: {plan:?}");
			assert!(!plan.iter().any(|(_, _, _, detail)| detail.contains("TEMP B-TREE")), "{query}: {plan:?}");
		}
	}

	#[tokio::test]
	async fn an_event_is_stored_once_and_never_replaced() {
		let pool = pool().await;
		let first = parked(S1, "e1", "2026-10-20T09:15:00.000Z");
		assert_eq!(put(&pool, A, &first).await.unwrap(), Change::Added);
		assert_eq!(put(&pool, A, &first).await.unwrap(), Change::Unchanged, "the same event writes nothing");
		let mut other = first.clone();
		other["type"] = json!("confirmation-taken");
		assert!(matches!(put(&pool, A, &other).await, Err(LogError::Rewritten)), "a different event under a held id");
		assert_eq!(rows(&pool).await, 1);
		assert_eq!(put(&pool, B, &other).await.unwrap(), Change::Added, "B's ids are B's");
		assert_eq!(
			put(&pool, A, &parked(S2, "e1", "2026-10-20T09:15:00.000Z")).await.unwrap(),
			Change::Added,
			"and a session's ids are its own"
		);
		let kinds: Vec<String> = sqlx::query_scalar("SELECT type FROM leetype_session_event ORDER BY rowid").fetch_all(&pool).await.unwrap();
		assert_eq!(kinds, ["parked", "confirmation-taken", "parked"], "the type column is the body's");

		let update = sqlx::query("UPDATE leetype_session_event SET body = '{}'").execute(&pool).await;
		assert!(update.unwrap_err().to_string().contains("append-only"), "no row is ever updated");
	}

	#[tokio::test]
	async fn a_session_reads_back_in_the_order_it_was_recorded() {
		let pool = pool().await;
		for event in [
			parked(S1, "e3", "2026-10-20T09:15:00.000Z"),
			started(S1, "e1", "2026-10-20T09:00:00.000Z"),
			parked(S1, "e2", "2026-10-20T09:15:00.000Z"),
			parked(S2, "e4", "2026-10-20T08:00:00.000Z"),
		] {
			put(&pool, A, &event).await.unwrap();
		}
		assert_eq!(ids(&pool, A, S1).await, ["e1", "e3", "e2"], "by instant, then as they arrived: e3 reached home before e2");
		let held = session_events(&mut pool.acquire().await.unwrap(), A, S1).await.unwrap();
		assert_eq!(
			serde_json::to_value(&held.events[0]).unwrap(),
			started(S1, "e1", "2026-10-20T09:00:00.000Z"),
			"the body as the phone wrote it"
		);
		assert!(ids(&pool, A, "never-seen").await.is_empty(), "a session home never saw is empty");
		assert!(ids(&pool, B, S1).await.is_empty(), "another subject's session is empty");
	}

	#[tokio::test]
	async fn the_sessions_list_newest_first_with_their_file_and_count() {
		let pool = pool().await;
		for event in [
			started(S1, "e1", "2026-10-20T09:00:00.000Z"),
			parked(S1, "e2", "2026-10-20T09:15:00.000Z"),
			parked(S2, "e3", "2026-10-20T10:00:00.000Z"),
			started(S2, "e4", "2026-10-20T09:30:00.000Z"),
			parked("s3", "e5", "2026-10-20T08:00:00.000Z"),
		] {
			put(&pool, A, &event).await.unwrap();
		}
		put(&pool, B, &started("s9", "e1", "2026-10-21T09:00:00.000Z")).await.unwrap();
		let listed = sessions(&mut pool.acquire().await.unwrap(), A).await.unwrap();
		assert_eq!(
			serde_json::to_value(&listed).unwrap(),
			json!({ "sessions": [
				{ "session": "s2", "file": "raft-1.json", "firstAt": "2026-10-20T09:30:00.000Z", "lastAt": "2026-10-20T10:00:00.000Z", "events": 2 },
				{ "session": "s1", "file": "raft-1.json", "firstAt": "2026-10-20T09:00:00.000Z", "lastAt": "2026-10-20T09:15:00.000Z", "events": 2 },
				{ "session": "s3", "file": null, "firstAt": "2026-10-20T08:00:00.000Z", "lastAt": "2026-10-20T08:00:00.000Z", "events": 1 },
			] }),
			"B's session is not A's, and a session with no `started` home holds names no file"
		);
	}

	#[tokio::test]
	async fn the_sessions_list_holds_the_newest_thousand() {
		let pool = pool().await;
		let total = usize::try_from(SESSIONS_LISTED).unwrap() + 1;
		let mut conn = pool.acquire().await.unwrap();
		let first = chrono::DateTime::parse_from_rfc3339("2026-10-20T00:00:00.000Z").unwrap();
		for n in 0..total {
			let session = String::from("s") + &n.to_string();
			let at = (first + chrono::TimeDelta::seconds(i64::try_from(n).unwrap())).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
			let event = started(&session, "e1", &at);
			put_event(&mut conn, A, &session, "e1", event.to_string().as_bytes(), NOW).await.unwrap();
		}
		let listed = sessions(&mut conn, A).await.unwrap().sessions;
		assert_eq!(listed.len(), total - 1);
		assert_eq!(listed[0].session, (String::from("s") + &(total - 1).to_string()), "the newest first");
		assert!(listed.iter().all(|session| session.session != "s0"), "the oldest is left out");
	}

	#[tokio::test]
	async fn a_refused_event_writes_nothing() {
		let pool = pool().await;
		let spoken = json!({ "id": "e1", "at": "2026-10-20T09:15:00.000Z", "session": S1, "type": "spoken", "file": "raft-1.json", "screen": "say-why",
			"answer": { "as": "recorded", "seconds": 41, "ticks": ["names the term"], "strong": true } });
		assert!(matches!(put(&pool, A, &spoken).await, Err(LogError::Invalid(problems)) if problems.iter().any(|(field, _)| *field == "answer.ticks")));
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
			put(&pool, subject, &parked(S1, "e1", "2026-10-20T09:15:00.000Z")).await.unwrap();
		}
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		let left: Vec<String> = sqlx::query_scalar("SELECT subject_id FROM leetype_session_event").fetch_all(&pool).await.unwrap();
		assert_eq!(left, [B], "A's record went with A's account, B's stayed");
	}
}
