//! The owner's Studio time log and friction reports (paulgsc/some-ui ST9,
//! #2257): each sitting's screens entered and left, the page hidden and
//! shown, the sitting opened and closed, and at its end how it went.
//!
//! `20261030000200_create_studio_log.up.sql` records the design; in short:
//!
//! - **Append-only.** [`put_event`] and [`put_report`] store a row under its
//!   id, or nothing when that id already holds the same body; a different
//!   body under a held id is refused ([`LogError::Rewritten`]), never stored.
//!   No row is ever updated (a trigger aborts an `UPDATE`), and nothing here
//!   deletes. Account deletion is the one delete: its generic sweep of
//!   `SUBJECT_SCOPED_TABLES`, and the migration's triggers on `account`.
//! - **The phone's rules, held here too.** A write passes [`parse_event`] or
//!   [`parse_report`], and every event of a sitting carries the sitting's
//!   one kind.
//! - **Per subject, and only per subject.** Every query takes the subject and
//!   filters on it; nothing reads across subjects.
//! - **No background writer.** The one caller is `file_host`'s
//!   `handlers::studio_log` (`PUT /api/v1/studio/sittings/:sitting/events/:id`
//!   and `PUT /api/v1/studio/friction/:id`). A
//!   `git grep -n -e studio_sitting_event -e studio_friction -- '*.rs'` shows
//!   no other writer.

mod model;

pub use model::{instant, parse_event, parse_report, Event, Happened, How, Kind, Problem, Report, BODY_CEILING, MAX_CAUSE_CHARS};
use serde::{de::DeserializeOwned, Serialize};
use sqlx::SqliteConnection;

/// The most sittings [`sittings`] lists: the newest.
pub const SITTINGS_LISTED: i64 = 1_000;

/// The most friction reports [`reports`] lists: the newest.
pub const REPORTS_LISTED: i64 = 1_000;

/// What a put did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
	/// Home did not hold it: it is stored now.
	Added,
	/// Home already holds this very body: nothing written.
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
	/// The id already holds a different body, which is never replaced.
	Rewritten,
	/// A stored body this crate wrote does not read back.
	Unreadable,
	Storage(sqlx::Error),
}

impl std::fmt::Display for LogError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Invalid(problems) => {
				f.write_str("not a write the Studio log takes:")?;
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
pub const REWRITTEN: &str = "this id already holds something different: the Studio log is append-only";

/// What [`LogError::Unreadable`] says.
pub const UNREADABLE: &str = "a stored Studio log entry does not read back";

/// One sitting's events as the record holds them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SittingEvents {
	pub sitting: String,
	/// In the order the phone recorded them: by `at`, then the order they
	/// reached home.
	pub events: Vec<Event>,
}

/// One sitting, summarised.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sitting {
	pub sitting: String,
	/// The sitting's kind, as each of its events carries it.
	pub kind: String,
	/// Its first and last event's `at`, as ISO-8601 UTC to the millisecond.
	pub first_at: String,
	pub last_at: String,
	/// How many events home holds for it.
	pub events: i64,
}

/// The sittings home holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Sittings {
	/// Newest first, by last event, at most [`SITTINGS_LISTED`].
	pub sittings: Vec<Sitting>,
}

/// The friction reports home holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reports {
	/// Newest first, by `at`, at most [`REPORTS_LISTED`].
	pub reports: Vec<Report>,
}

fn read_body<T: DeserializeOwned>(body: &str) -> Result<T, LogError> {
	serde_json::from_str(body).map_err(|_| LogError::Unreadable)
}

fn text<T: Serialize>(value: &T) -> Result<String, LogError> {
	serde_json::to_value(value).map(|json| json.to_string()).map_err(|_| LogError::Unreadable)
}

/// Whether a held body answers a put of `sent`: the same is unchanged, any
/// other is refused.
fn standing<T: DeserializeOwned + PartialEq>(held: &str, sent: T) -> Result<(Change, T), LogError> {
	if read_body::<T>(held)? == sent {
		Ok((Change::Unchanged, sent))
	} else {
		Err(LogError::Rewritten)
	}
}

/// Stores `body` as event `id` of `sitting` for `subject`, as of `now`, and
/// answers what it did with the event as it now stands.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the reads that decide and
/// the write see one snapshot.
///
/// # Errors
/// [`LogError::Invalid`] naming every problem [`parse_event`] finds, or a
/// `kind` other than the one the sitting's held events carry;
/// [`LogError::Rewritten`] when `id` already holds a different event in
/// `sitting`; and [`LogError::Storage`] for any `sqlx` failure. A refusal is
/// decided before anything is written.
pub async fn put_event(conn: &mut SqliteConnection, subject: &str, sitting: &str, id: &str, body: &[u8], now: &str) -> Result<(Change, Event), LogError> {
	let event = parse_event(sitting, id, body).map_err(LogError::Invalid)?;
	let held = sqlx::query_scalar!(
		"SELECT body FROM studio_sitting_event WHERE subject_id = ?1 AND sitting_id = ?2 AND event_id = ?3",
		subject,
		sitting,
		id
	)
	.fetch_optional(&mut *conn)
	.await?;
	if let Some(held) = held {
		return standing(&held, event);
	}
	let kind = event.kind.as_str();
	let held_kind = sqlx::query_scalar!("SELECT kind FROM studio_sitting_event WHERE subject_id = ?1 AND sitting_id = ?2 LIMIT 1", subject, sitting)
		.fetch_optional(&mut *conn)
		.await?;
	if held_kind.is_some_and(|held| held != kind) {
		return Err(LogError::Invalid(vec![("kind", "must be the sitting's kind, as its earlier events carry it")]));
	}
	let at = instant(&event.at).ok_or(LogError::Unreadable)?;
	let happened = event.happened.as_str();
	let body = text(&event)?;
	sqlx::query!(
		"INSERT INTO studio_sitting_event (subject_id, sitting_id, event_id, at, type, kind, body, written_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
		subject,
		sitting,
		id,
		at,
		happened,
		kind,
		body,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok((Change::Added, event))
}

/// `sitting`'s events for `subject`, as they stand: none for a sitting home
/// has never seen.
///
/// Unbounded, as `leetype_log_repo::session_events` is: one sitting is a few
/// dozen events, and the phone reads them back whole to confirm what home
/// holds before it lets them go.
///
/// # Errors
/// [`LogError::Unreadable`] for a stored body that does not read back, and
/// [`LogError::Storage`] for any `sqlx` failure.
pub async fn sitting_events(conn: &mut SqliteConnection, subject: &str, sitting: &str) -> Result<SittingEvents, LogError> {
	let bodies = sqlx::query_scalar!(
		"SELECT body FROM studio_sitting_event WHERE subject_id = ?1 AND sitting_id = ?2 ORDER BY at, rowid",
		subject,
		sitting
	)
	.fetch_all(&mut *conn)
	.await?;
	let events = bodies.iter().map(|body| read_body(body)).collect::<Result<Vec<Event>, LogError>>()?;
	Ok(SittingEvents {
		sitting: sitting.to_owned(),
		events,
	})
}

/// The newest [`SITTINGS_LISTED`] of `subject`'s sittings, newest first.
///
/// # Errors
/// [`LogError::Storage`] for any `sqlx` failure.
pub async fn sittings(conn: &mut SqliteConnection, subject: &str) -> Result<Sittings, LogError> {
	let rows = sqlx::query!(
		r#"SELECT
			sitting_id AS "sitting!: String",
			MIN(kind) AS "kind!: String",
			MIN(at) AS "first_at!: String",
			MAX(at) AS "last_at!: String",
			COUNT(*) AS "events!: i64"
		FROM studio_sitting_event
		WHERE subject_id = ?1
		GROUP BY sitting_id
		ORDER BY MAX(at) DESC, sitting_id
		LIMIT ?2"#,
		subject,
		SITTINGS_LISTED
	)
	.fetch_all(&mut *conn)
	.await?;
	let sittings = rows
		.into_iter()
		.map(|row| Sitting {
			sitting: row.sitting,
			kind: row.kind,
			first_at: row.first_at,
			last_at: row.last_at,
			events: row.events,
		})
		.collect();
	Ok(Sittings { sittings })
}

/// Stores `body` as friction report `id` for `subject`, as of `now`, and
/// answers what it did with the report as it now stands.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, as [`put_event`].
///
/// # Errors
/// [`LogError::Invalid`] naming every problem [`parse_report`] finds,
/// [`LogError::Rewritten`] when `id` already holds a different report, and
/// [`LogError::Storage`] for any `sqlx` failure. A refusal is decided before
/// anything is written.
pub async fn put_report(conn: &mut SqliteConnection, subject: &str, id: &str, body: &[u8], now: &str) -> Result<(Change, Report), LogError> {
	let report = parse_report(id, body).map_err(LogError::Invalid)?;
	let held = sqlx::query_scalar!("SELECT body FROM studio_friction WHERE subject_id = ?1 AND report_id = ?2", subject, id)
		.fetch_optional(&mut *conn)
		.await?;
	if let Some(held) = held {
		return standing(&held, report);
	}
	let at = instant(&report.at).ok_or(LogError::Unreadable)?;
	let how = report.how.as_str();
	let body = text(&report)?;
	sqlx::query!(
		"INSERT INTO studio_friction (subject_id, report_id, sitting_id, at, how, body, written_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
		subject,
		id,
		report.sitting,
		at,
		how,
		body,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok((Change::Added, report))
}

/// The newest [`REPORTS_LISTED`] of `subject`'s friction reports, newest
/// first.
///
/// # Errors
/// [`LogError::Unreadable`] for a stored body that does not read back, and
/// [`LogError::Storage`] for any `sqlx` failure.
pub async fn reports(conn: &mut SqliteConnection, subject: &str) -> Result<Reports, LogError> {
	let bodies = sqlx::query_scalar!(
		"SELECT body FROM studio_friction WHERE subject_id = ?1 ORDER BY at DESC, rowid DESC LIMIT ?2",
		subject,
		REPORTS_LISTED
	)
	.fetch_all(&mut *conn)
	.await?;
	let reports = bodies.iter().map(|body| read_body(body)).collect::<Result<Vec<Report>, LogError>>()?;
	Ok(Reports { reports })
}

#[cfg(test)]
mod tests {
	use super::{put_event, put_report, reports, sitting_events, sittings, Change, LogError, REPORTS_LISTED, SITTINGS_LISTED};
	use serde_json::{json, Value};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const PREVIOUS_MIGRATION: i64 = 20_261_030_000_100;
	const NOW: &str = "2026-10-30T23:00:00.000Z";
	const A: &str = "subject-a";
	const B: &str = "subject-b";
	const S1: &str = "s1";
	const S2: &str = "s2";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn entered(sitting: &str, id: &str, at: &str) -> Value {
		json!({ "id": id, "at": at, "sitting": sitting, "kind": "record", "type": "screen_entered", "screen": "record.shot-list" })
	}

	fn hidden(sitting: &str, id: &str, at: &str) -> Value {
		json!({ "id": id, "at": at, "sitting": sitting, "kind": "record", "type": "page_hidden", "screen": null })
	}

	fn friction(id: &str, at: &str, how: &str) -> Value {
		json!({ "id": id, "at": at, "sitting": S1, "kind": "record", "how": how, "cause": "the mic clipped", "shot": "ep04:s3", "voice": null })
	}

	async fn put(pool: &SqlitePool, subject: &str, value: &Value) -> Result<Change, LogError> {
		let (sitting, id) = (value["sitting"].as_str().unwrap(), value["id"].as_str().unwrap());
		put_event(&mut pool.acquire().await.unwrap(), subject, sitting, id, value.to_string().as_bytes(), NOW)
			.await
			.map(|(change, _)| change)
	}

	async fn report(pool: &SqlitePool, subject: &str, value: &Value) -> Result<Change, LogError> {
		put_report(
			&mut pool.acquire().await.unwrap(),
			subject,
			value["id"].as_str().unwrap(),
			value.to_string().as_bytes(),
			NOW,
		)
		.await
		.map(|(change, _)| change)
	}

	async fn count(pool: &SqlitePool, table: &str) -> i64 {
		sqlx::query_scalar(&(String::from("SELECT COUNT(*) FROM ") + table)).fetch_one(pool).await.unwrap()
	}

	async fn ids(pool: &SqlitePool, subject: &str, sitting: &str) -> Vec<String> {
		let held = sitting_events(&mut pool.acquire().await.unwrap(), subject, sitting).await.unwrap();
		assert_eq!(held.sitting, sitting);
		held.events.iter().map(|event| event.id.clone()).collect()
	}

	async fn report_ids(pool: &SqlitePool, subject: &str) -> Vec<String> {
		reports(&mut pool.acquire().await.unwrap(), subject)
			.await
			.unwrap()
			.reports
			.into_iter()
			.map(|report| report.id)
			.collect()
	}

	#[tokio::test]
	async fn the_migration_round_trips_and_refuses_to_drop_a_record() {
		let pool = pool().await;
		let tables = || sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('studio_sitting_event', 'studio_friction')");
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 2);
		MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 0, "down drops both while they are empty");
		MIGRATOR.run(&pool).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 2, "up recreates them");

		report(&pool, A, &friction("f1", "2026-10-30T10:00:00.000Z", "rough")).await.unwrap();
		assert!(MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.is_err(), "a held report refuses the rollback");
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 2, "and leaves both as they were");
		assert_eq!(count(&pool, "studio_friction").await, 1);
	}

	/// The columns the migration describes and no others: a new one is a
	/// decision about what the server learns from the record.
	#[tokio::test]
	async fn the_tables_hold_exactly_their_described_columns() {
		let pool = pool().await;
		let columns = |table: &str| sqlx::query_scalar::<_, String>("SELECT name FROM pragma_table_info(?1) ORDER BY cid").bind(table.to_owned());
		assert_eq!(
			columns("studio_sitting_event").fetch_all(&pool).await.unwrap(),
			["subject_id", "sitting_id", "event_id", "at", "type", "kind", "body", "written_at"]
		);
		assert_eq!(
			columns("studio_friction").fetch_all(&pool).await.unwrap(),
			["subject_id", "report_id", "sitting_id", "at", "how", "body", "written_at"]
		);
	}

	/// Every read goes by an index on the subject, with no scan and no sort.
	/// The queries mirror `put_event`, `sitting_events`, `put_report` and
	/// `reports`, with literals for their binds.
	#[tokio::test]
	async fn every_read_goes_by_the_subject() {
		let pool = pool().await;
		for (query, index) in [
			(
				"EXPLAIN QUERY PLAN SELECT body FROM studio_sitting_event WHERE subject_id = 'a' AND sitting_id = 's' AND event_id = 'e'",
				"sqlite_autoindex_studio_sitting_event_1 (subject_id=? AND sitting_id=? AND event_id=?)",
			),
			(
				"EXPLAIN QUERY PLAN SELECT body FROM studio_sitting_event WHERE subject_id = 'a' AND sitting_id = 's' ORDER BY at, rowid",
				"studio_sitting_event_order (subject_id=? AND sitting_id=?)",
			),
			(
				"EXPLAIN QUERY PLAN SELECT body FROM studio_friction WHERE subject_id = 'a' AND report_id = 'f'",
				"sqlite_autoindex_studio_friction_1 (subject_id=? AND report_id=?)",
			),
			(
				"EXPLAIN QUERY PLAN SELECT body FROM studio_friction WHERE subject_id = 'a' ORDER BY at DESC, rowid DESC LIMIT 1000",
				"studio_friction_order (subject_id=?)",
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
		let first = entered(S1, "e1", "2026-10-30T09:15:00.000Z");
		assert_eq!(put(&pool, A, &first).await.unwrap(), Change::Added);
		assert_eq!(put(&pool, A, &first).await.unwrap(), Change::Unchanged, "a retried send writes nothing");
		let mut other = first.clone();
		other["screen"] = json!("record.take");
		assert!(matches!(put(&pool, A, &other).await, Err(LogError::Rewritten)), "a different event under a held id");
		assert_eq!(count(&pool, "studio_sitting_event").await, 1);
		assert_eq!(put(&pool, B, &other).await.unwrap(), Change::Added, "B's ids are B's");
		assert_eq!(
			put(&pool, A, &hidden(S2, "e1", "2026-10-30T09:15:00.000Z")).await.unwrap(),
			Change::Added,
			"and a sitting's ids are its own"
		);
		let columns: Vec<(String, String)> = sqlx::query_as("SELECT type, kind FROM studio_sitting_event ORDER BY rowid").fetch_all(&pool).await.unwrap();
		assert_eq!(
			columns,
			[
				("screen_entered".into(), "record".into()),
				("screen_entered".into(), "record".into()),
				("page_hidden".into(), "record".into())
			],
			"the columns are the body's"
		);

		let update = sqlx::query("UPDATE studio_sitting_event SET body = '{}'").execute(&pool).await;
		assert!(update.unwrap_err().to_string().contains("append-only"), "no row is ever updated");
	}

	#[tokio::test]
	async fn a_sitting_keeps_one_kind() {
		let pool = pool().await;
		put(&pool, A, &entered(S1, "e1", "2026-10-30T09:15:00.000Z")).await.unwrap();
		let mut watch = hidden(S1, "e2", "2026-10-30T09:16:00.000Z");
		watch["kind"] = json!("watch");
		assert!(matches!(put(&pool, A, &watch).await, Err(LogError::Invalid(problems)) if problems == [("kind", "must be the sitting's kind, as its earlier events carry it")]));
		assert_eq!(put(&pool, B, &watch).await.unwrap(), Change::Added, "another subject's sitting is its own");
		assert_eq!(count(&pool, "studio_sitting_event").await, 2);
	}

	#[tokio::test]
	async fn a_sitting_reads_back_in_the_order_it_was_recorded() {
		let pool = pool().await;
		for event in [
			hidden(S1, "e3", "2026-10-30T09:15:00.000Z"),
			entered(S1, "e1", "2026-10-30T09:00:00.000Z"),
			hidden(S1, "e2", "2026-10-30T09:15:00.000Z"),
			hidden(S2, "e4", "2026-10-30T08:00:00.000Z"),
		] {
			put(&pool, A, &event).await.unwrap();
		}
		assert_eq!(ids(&pool, A, S1).await, ["e1", "e3", "e2"], "by instant, then as they arrived");
		let held = sitting_events(&mut pool.acquire().await.unwrap(), A, S1).await.unwrap();
		assert_eq!(serde_json::to_value(&held.events[0]).unwrap(), entered(S1, "e1", "2026-10-30T09:00:00.000Z"));
		assert!(ids(&pool, A, "never-seen").await.is_empty(), "a sitting home never saw is empty");
		assert!(ids(&pool, B, S1).await.is_empty(), "another subject's sitting is empty");
	}

	#[tokio::test]
	async fn the_sittings_list_newest_first_with_their_kind_and_count() {
		let pool = pool().await;
		let mut watched = hidden(S2, "e3", "2026-10-30T10:00:00.000Z");
		watched["kind"] = json!("watch");
		for event in [entered(S1, "e1", "2026-10-30T09:00:00.000Z"), hidden(S1, "e2", "2026-10-30T09:15:00.000Z"), watched] {
			put(&pool, A, &event).await.unwrap();
		}
		put(&pool, B, &entered("s9", "e1", "2026-10-31T09:00:00.000Z")).await.unwrap();
		let listed = sittings(&mut pool.acquire().await.unwrap(), A).await.unwrap();
		assert_eq!(
			serde_json::to_value(&listed).unwrap(),
			json!({ "sittings": [
				{ "sitting": "s2", "kind": "watch", "firstAt": "2026-10-30T10:00:00.000Z", "lastAt": "2026-10-30T10:00:00.000Z", "events": 1 },
				{ "sitting": "s1", "kind": "record", "firstAt": "2026-10-30T09:00:00.000Z", "lastAt": "2026-10-30T09:15:00.000Z", "events": 2 },
			] }),
			"B's sitting is not A's"
		);
	}

	#[tokio::test]
	async fn the_lists_hold_the_newest_thousand() {
		let pool = pool().await;
		let total = usize::try_from(SITTINGS_LISTED.max(REPORTS_LISTED)).unwrap() + 1;
		let mut conn = pool.acquire().await.unwrap();
		let first = chrono::DateTime::parse_from_rfc3339("2026-10-30T00:00:00.000Z").unwrap();
		for n in 0..total {
			let name = n.to_string();
			let at = (first + chrono::TimeDelta::seconds(i64::try_from(n).unwrap())).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
			let sitting = String::from("s") + &name;
			put_event(&mut conn, A, &sitting, "e1", entered(&sitting, "e1", &at).to_string().as_bytes(), NOW)
				.await
				.unwrap();
			let id = String::from("f") + &name;
			put_report(&mut conn, A, &id, friction(&id, &at, "smooth").to_string().as_bytes(), NOW).await.unwrap();
		}
		let listed = sittings(&mut conn, A).await.unwrap().sittings;
		assert_eq!(listed.len(), usize::try_from(SITTINGS_LISTED).unwrap());
		assert_eq!(listed[0].sitting, String::from("s") + &(total - 1).to_string(), "the newest first");
		assert!(listed.iter().all(|sitting| sitting.sitting != "s0"), "the oldest is left out");
		let held = reports(&mut conn, A).await.unwrap().reports;
		assert_eq!(held.len(), usize::try_from(REPORTS_LISTED).unwrap());
		assert_eq!(held[0].id, String::from("f") + &(total - 1).to_string(), "the newest first");
		assert!(held.iter().all(|report| report.id != "f0"), "the oldest is left out");
	}

	#[tokio::test]
	async fn a_report_is_stored_once_and_never_replaced() {
		let pool = pool().await;
		let first = friction("f1", "2026-10-30T10:00:00.000Z", "rough");
		assert_eq!(report(&pool, A, &first).await.unwrap(), Change::Added);
		assert_eq!(report(&pool, A, &first).await.unwrap(), Change::Unchanged, "a retried send writes nothing");
		let mut other = first.clone();
		other["how"] = json!("smooth");
		assert!(matches!(report(&pool, A, &other).await, Err(LogError::Rewritten)));
		assert_eq!(report(&pool, B, &other).await.unwrap(), Change::Added, "B's ids are B's");
		report(&pool, A, &friction("f2", "2026-10-30T11:00:00.000Z", "some_snags")).await.unwrap();
		report(&pool, A, &friction("f3", "2026-10-30T10:00:00.000Z", "smooth")).await.unwrap();
		assert_eq!(report_ids(&pool, A).await, ["f2", "f3", "f1"], "newest first, then the later arrival first");
		assert_eq!(report_ids(&pool, B).await, ["f1"]);
		let held = reports(&mut pool.acquire().await.unwrap(), A).await.unwrap();
		assert_eq!(serde_json::to_value(&held.reports[2]).unwrap(), first, "the body as the phone wrote it");
		let columns: Vec<(String, String, String)> = sqlx::query_as("SELECT report_id, sitting_id, how FROM studio_friction WHERE subject_id = 'subject-a' ORDER BY rowid")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(columns[0], ("f1".into(), "s1".into(), "rough".into()), "the columns are the body's");

		let update = sqlx::query("UPDATE studio_friction SET body = '{}'").execute(&pool).await;
		assert!(update.unwrap_err().to_string().contains("append-only"), "no row is ever updated");
	}

	#[tokio::test]
	async fn a_refused_write_writes_nothing() {
		let pool = pool().await;
		let mut event = entered(S1, "e1", "2026-10-30T09:15:00.000Z");
		event["screen"] = Value::Null;
		assert!(matches!(put(&pool, A, &event).await, Err(LogError::Invalid(problems)) if problems.iter().any(|(field, _)| *field == "screen")));
		let mut long = friction("f1", "2026-10-30T10:00:00.000Z", "rough");
		long["cause"] = json!("x".repeat(201));
		assert!(matches!(report(&pool, A, &long).await, Err(LogError::Invalid(problems)) if problems.iter().any(|(field, _)| *field == "cause")));
		assert_eq!(count(&pool, "studio_sitting_event").await + count(&pool, "studio_friction").await, 0);
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
			put(&pool, subject, &entered(S1, "e1", "2026-10-30T09:15:00.000Z")).await.unwrap();
			report(&pool, subject, &friction("f1", "2026-10-30T10:00:00.000Z", "rough")).await.unwrap();
		}
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		for table in ["studio_sitting_event", "studio_friction"] {
			let left: Vec<String> = sqlx::query_scalar(&(String::from("SELECT subject_id FROM ") + table)).fetch_all(&pool).await.unwrap();
			assert_eq!(left, [B], "{table}: A's record went with A's account, B's stayed");
		}
	}
}
