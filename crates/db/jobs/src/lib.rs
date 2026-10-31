//! The owner's Jobs record (paulgsc/some-ui `apps/www/src/lib/jobs`): an
//! append-only log of what the owner did with each day's job picks, and what
//! came of it.
//!
//! `20261018000100_create_jobs_record.up.sql` records the design; in short:
//!
//! - **Append-only.** [`put_event`] stores an event under its id, or nothing
//!   when that id already holds the same event; a different body under a
//!   held id is refused ([`JobsError::Rewritten`]), never stored. No row is
//!   ever updated (a trigger aborts an `UPDATE`), and nothing here deletes.
//!   Account deletion is the one delete: its generic sweep of
//!   `SUBJECT_SCOPED_TABLES`, and the migration's trigger on `account`.
//! - **The phone's rules, held here too.** A write passes [`parse_event`]:
//!   the event's shape and each field's, as the phone records them.
//! - **No audio.** An event names its voice notes by the phone's ids; the
//!   notes themselves stay on the phone and in the owner's Drive.
//! - **Per subject, and only per subject.** Every query takes the subject and
//!   filters on it; nothing reads across subjects.
//! - **No background writer.** The callers are `file_host`'s `handlers::jobs`
//!   (`PUT /api/v1/jobs/events/:id`) and `handlers::sync`
//!   (`POST /api/v1/sync/jobs/batch`, each event as that route), which the
//!   phone sends once paired. A
//!   `git grep -n jobs_event -- '*.rs'` shows no other writer.

mod model;

pub use model::{instant, parse_event, DebriefResult, Event, Family, Friction, Outcome, PassReason, PostedOn, Problem, Scout, ShareKind, Track, EVENT_BODY_CEILING};
use serde::Serialize;
use sqlx::SqliteConnection;

/// What a put did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
	/// The record did not hold it: it is stored now.
	Added,
	/// The record already holds this very event: nothing written.
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
pub enum JobsError {
	/// The write breaks a rule; every problem, by field.
	Invalid(Vec<Problem>),
	/// The id already holds a different event, which is never replaced.
	Rewritten,
	/// A stored body this crate wrote does not read back.
	Unreadable,
	Storage(sqlx::Error),
}

impl std::fmt::Display for JobsError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Invalid(problems) => {
				f.write_str("not a write the Jobs record takes:")?;
				for (field, why) in problems {
					write!(f, " {field} {why};")?;
				}
				Ok(())
			}
			Self::Rewritten => f.write_str(REWRITTEN),
			Self::Unreadable => f.write_str("a stored Jobs event does not read back"),
			Self::Storage(err) => write!(f, "database error: {err}"),
		}
	}
}

impl std::error::Error for JobsError {}

impl From<sqlx::Error> for JobsError {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

/// What [`JobsError::Rewritten`] says.
pub const REWRITTEN: &str = "this event id already holds a different event: the Jobs record is append-only";

/// The record as it stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Record {
	/// In the order the phone recorded them: by `at`, then the order they
	/// reached home, which is the phone's log order (its outbox sends in order).
	pub events: Vec<Event>,
}

fn read_body(body: &str) -> Result<Event, JobsError> {
	serde_json::from_str(body).map_err(|_| JobsError::Unreadable)
}

/// Stores `body` as event `id` for `subject`, as of `now`, and answers what
/// it did with the event as it now stands.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the read that decides and
/// the write see one snapshot.
///
/// # Errors
/// [`JobsError::Invalid`] naming every problem [`parse_event`] finds,
/// [`JobsError::Rewritten`] when `id` already holds a different event, and
/// [`JobsError::Storage`] for any `sqlx` failure. A refusal is decided before
/// anything is written, so a caller that goes on after one (a batch,
/// `file_host`'s `handlers::sync`) keeps nothing of it.
pub async fn put_event(conn: &mut SqliteConnection, subject: &str, id: &str, body: &[u8], now: &str) -> Result<(Change, Event), JobsError> {
	let event = parse_event(id, body).map_err(JobsError::Invalid)?;
	let standing = sqlx::query_scalar!("SELECT body FROM jobs_event WHERE subject_id = ?1 AND event_id = ?2", subject, id)
		.fetch_optional(&mut *conn)
		.await?;
	if let Some(standing) = standing {
		return if read_body(&standing)? == event {
			Ok((Change::Unchanged, event))
		} else {
			Err(JobsError::Rewritten)
		};
	}
	let at = instant(event.at()).ok_or(JobsError::Unreadable)?;
	let text = serde_json::to_value(&event).map(|json| json.to_string()).map_err(|_| JobsError::Unreadable)?;
	sqlx::query!(
		"INSERT INTO jobs_event (subject_id, event_id, at, body, written_at) VALUES (?1, ?2, ?3, ?4, ?5)",
		subject,
		id,
		at,
		text,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok((Change::Added, event))
}

/// `subject`'s record as it stands.
///
/// Unbounded on purpose: it is one person's log, a few dozen events a day at
/// most, and the phone reads it whole to rebuild its own.
///
/// # Errors
/// [`JobsError::Unreadable`] for a stored body that does not read back, and
/// [`JobsError::Storage`] for any `sqlx` failure.
pub async fn record(conn: &mut SqliteConnection, subject: &str) -> Result<Record, JobsError> {
	let bodies = sqlx::query_scalar!("SELECT body FROM jobs_event WHERE subject_id = ?1 ORDER BY at, rowid", subject)
		.fetch_all(&mut *conn)
		.await?;
	let events = bodies.iter().map(|body| read_body(body)).collect::<Result<Vec<Event>, JobsError>>()?;
	Ok(Record { events })
}

#[cfg(test)]
mod tests {
	use super::{put_event, record, Change, JobsError};
	use serde_json::{json, Value};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const PREVIOUS_MIGRATION: i64 = 20_261_017_000_100;
	const NOW: &str = "2026-10-18T23:00:00.000Z";
	const A: &str = "subject-a";
	const B: &str = "subject-b";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn taken(id: &str, at: &str) -> Value {
		json!({
			"id": id, "at": at, "type": "taken",
			"day": "2026-10-18", "pickId": "blu-7f2a", "fingerprint": "bluesight|data analyst|remote"
		})
	}

	async fn put(pool: &SqlitePool, subject: &str, value: &Value) -> Result<Change, JobsError> {
		let id = value["id"].as_str().unwrap();
		put_event(&mut pool.acquire().await.unwrap(), subject, id, value.to_string().as_bytes(), NOW)
			.await
			.map(|(change, _)| change)
	}

	async fn rows(pool: &SqlitePool) -> i64 {
		sqlx::query_scalar("SELECT COUNT(*) FROM jobs_event").fetch_one(pool).await.unwrap()
	}

	#[tokio::test]
	async fn the_migration_round_trips() {
		let pool = pool().await;
		let tables = || sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'jobs_event'");
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
		let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('jobs_event') ORDER BY cid")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(columns, ["subject_id", "event_id", "at", "body", "written_at"]);
	}

	#[tokio::test]
	async fn an_event_is_stored_once_and_never_replaced() {
		let pool = pool().await;
		let first = taken("e1", "2026-10-18T22:10:00.000Z");
		assert_eq!(put(&pool, A, &first).await.unwrap(), Change::Added);
		assert_eq!(put(&pool, A, &first).await.unwrap(), Change::Unchanged, "the same event writes nothing");
		let mut other = first.clone();
		other["type"] = json!("resume-ready");
		assert!(matches!(put(&pool, A, &other).await, Err(JobsError::Rewritten)), "a different event under a held id");
		assert_eq!(rows(&pool).await, 1);
		assert_eq!(put(&pool, B, &other).await.unwrap(), Change::Added, "B's ids are B's");

		let update = sqlx::query("UPDATE jobs_event SET body = '{}'").execute(&pool).await;
		assert!(update.unwrap_err().to_string().contains("append-only"), "no row is ever updated");
	}

	#[tokio::test]
	async fn the_record_reads_back_in_the_order_it_was_recorded() {
		let pool = pool().await;
		for event in [
			taken("e3", "2026-10-18T22:10:00.000Z"),
			taken("e1", "2026-10-18T15:20:00-07:00"),
			taken("e2", "2026-10-18T22:10:00.000Z"),
		] {
			put(&pool, A, &event).await.unwrap();
		}
		let held = record(&mut pool.acquire().await.unwrap(), A).await.unwrap();
		let ids: Vec<&str> = held.events.iter().map(super::Event::id).collect();
		assert_eq!(ids, ["e3", "e2", "e1"], "by instant in UTC, then as they arrived: e3 reached home before e2");
		assert_eq!(
			serde_json::to_value(&held.events[2]).unwrap(),
			taken("e1", "2026-10-18T15:20:00-07:00"),
			"the body as the phone wrote it"
		);
		assert!(
			record(&mut pool.acquire().await.unwrap(), B).await.unwrap().events.is_empty(),
			"another subject's record is empty"
		);
	}

	/// A scout's posted day left out and `null` are different events: each
	/// is stored and read back as sent, and one never stands in for the other.
	#[tokio::test]
	async fn a_scouts_unsaid_and_undated_posted_day_stay_apart() {
		let pool = pool().await;
		let scout = |id: &str, extra: Value| {
			let mut event = json!({ "id": id, "at": "2026-10-18T22:10:00.000Z", "type": "scout", "voice": [], "scout": {
				"id": "sc-a1b2c3d4", "url": "https://www.indeed.com/viewjob?jk=0123", "title": "SDET",
				"why": "", "postingText": null, "track": "software", "family": null
			} });
			for (key, value) in extra.as_object().unwrap() {
				event["scout"][key] = value.clone();
			}
			event
		};
		let unsaid = scout("e1", json!({}));
		let undated = scout("e2", json!({ "location": "Remote", "postedOn": null }));
		let dated = scout("e3", json!({ "postedOn": "2026-10-15" }));
		for event in [&unsaid, &undated, &dated] {
			assert_eq!(put(&pool, A, event).await.unwrap(), Change::Added);
			assert_eq!(put(&pool, A, event).await.unwrap(), Change::Unchanged, "a resend writes nothing");
		}
		let mut now_undated = unsaid.clone();
		now_undated["scout"]["postedOn"] = Value::Null;
		assert!(matches!(put(&pool, A, &now_undated).await, Err(JobsError::Rewritten)), "unsaid is not later made undated");
		let held = record(&mut pool.acquire().await.unwrap(), A).await.unwrap();
		let bodies: Vec<Value> = held.events.iter().map(|event| serde_json::to_value(event).unwrap()).collect();
		assert_eq!(bodies, [unsaid, undated, dated], "each as the phone sent it");
	}

	#[tokio::test]
	async fn a_refused_event_writes_nothing() {
		let pool = pool().await;
		let mut bad = taken("e1", "2026-10-18T22:10:00.000Z");
		bad["fingerprint"] = json!("one part");
		assert!(matches!(put(&pool, A, &bad).await, Err(JobsError::Invalid(problems)) if problems.iter().any(|(field, _)| *field == "fingerprint")));
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
			put(&pool, subject, &taken("e1", "2026-10-18T22:10:00.000Z")).await.unwrap();
		}
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		let left: Vec<String> = sqlx::query_scalar("SELECT subject_id FROM jobs_event").fetch_all(&pool).await.unwrap();
		assert_eq!(left, [B], "A's record went with A's account, B's stayed");
	}
}
