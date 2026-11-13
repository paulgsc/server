//! What the app saw happen as it ran (paulgsc/some-ui CK2, #2381): one event
//! per obligation the phone ran to an ending, kept on home, and a summary of
//! them per process family and screen for the page that shows what works
//! (CK8, paulgsc/some-ui#2388).
//!
//! `20261103001700_create_app_event.up.sql` records the design; in short:
//!
//! - **One table for every family.** An event's `family` says which shape
//!   its body has ([`Event`]); a new family is a variant there, riding the
//!   same route and the same rows (paulgsc/some-ui ENG2, #2535).
//! - **Append-only.** [`put_event`] stores a row under its id, or nothing
//!   when that id already holds the same body; a different body under a held
//!   id is refused ([`EventError::Rewritten`]), never stored. No row is ever
//!   updated (a trigger aborts an `UPDATE`), and nothing here deletes.
//!   Account deletion is the one delete: its generic sweep of
//!   `SUBJECT_SCOPED_TABLES`, and the migration's trigger on `account`.
//! - **Ids only.** Every text field is a name from the app's code
//!   ([`parse_event`]), so nothing typed or said is stored (the owner's C3).
//! - **Per subject, and only per subject.** Every query takes the subject and
//!   filters on it; nothing reads across subjects.
//! - **A period read for the weekly report.** [`weekly`] counts one ISO
//!   week's engagement beside the week before, a quarter ago and a year ago
//!   (paulgsc/some-ui ENG12, #2700); see `weeks`.
//! - **No background writer.** The one caller is `file_host`'s
//!   `handlers::app_events` (`PUT /api/v1/app-events/:id`). A
//!   `git grep -n app_event -- '*.rs'` shows no other writer.

mod model;
mod weeks;

use std::collections::BTreeMap;

pub use model::{instant, parse_event, Event, Outcome, Problem, Went, BODY_CEILING, MAX_MS};
use serde::Serialize;
use sqlx::SqliteConnection;
pub use weeks::{weekly, IsoWeek, Role, Seen, Week, WeekUse, Weekly, ZONE};

/// The most events of a family one summary reads from its window: the
/// newest. A take a minute, all day, for a month is about 43,000; past this
/// the summary says it is `partial`.
pub const WINDOW_READ: i64 = 50_000;

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

/// Why a put or a read did nothing.
#[derive(Debug)]
pub enum EventError {
	/// The write breaks a rule; every problem, by field.
	Invalid(Vec<Problem>),
	/// The id already holds a different body, which is never replaced.
	Rewritten,
	/// A stored body this crate wrote does not read back.
	Unreadable,
	Storage(sqlx::Error),
}

impl std::fmt::Display for EventError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Invalid(problems) => {
				f.write_str("not an app event home takes:")?;
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

impl std::error::Error for EventError {}

impl From<sqlx::Error> for EventError {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

/// What [`EventError::Rewritten`] says.
pub const REWRITTEN: &str = "this id already holds a different event: app events are append-only";

/// What [`EventError::Unreadable`] says.
pub const UNREADABLE: &str = "a stored app event does not read back";

fn read_body(body: &str) -> Result<Event, EventError> {
	serde_json::from_str(body).map_err(|_| EventError::Unreadable)
}

/// Stores `body` as event `id` for `subject`, as of `now`, and answers what
/// it did with the event as it now stands.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the read that decides and
/// the write see one snapshot.
///
/// # Errors
/// [`EventError::Invalid`] naming every problem [`parse_event`] finds,
/// [`EventError::Rewritten`] when `id` already holds a different event, and
/// [`EventError::Storage`] for any `sqlx` failure. A refusal is decided
/// before anything is written.
pub async fn put_event(conn: &mut SqliteConnection, subject: &str, id: &str, body: &[u8], now: &str) -> Result<(Change, Event), EventError> {
	let event = parse_event(id, body).map_err(EventError::Invalid)?;
	let held = sqlx::query_scalar!("SELECT body FROM app_event WHERE subject_id = ?1 AND event_id = ?2", subject, id)
		.fetch_optional(&mut *conn)
		.await?;
	if let Some(held) = held {
		return if read_body(&held)? == event {
			Ok((Change::Unchanged, event))
		} else {
			Err(EventError::Rewritten)
		};
	}
	let at = instant(event.at()).ok_or(EventError::Unreadable)?;
	let family = event.family();
	let text = serde_json::to_value(&event).map(|json| json.to_string()).map_err(|_| EventError::Unreadable)?;
	sqlx::query!(
		"INSERT INTO app_event (subject_id, event_id, family, at, body, written_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
		subject,
		id,
		family,
		at,
		text,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok((Change::Added, event))
}

/// How many events of one family home holds in the window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FamilyCount {
	pub family: String,
	pub events: i64,
}

/// How long a step took over the window's outcomes that said, in
/// milliseconds: the median and the longest, and of how many.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Timing {
	pub median: u32,
	pub max: u32,
	pub of: usize,
}

/// One process family on one screen over the window: how its outcomes went,
/// and how long the ask took to be ready.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenOutcomes {
	pub process: String,
	pub screen: String,
	pub worked: usize,
	pub failed: usize,
	pub stopped: usize,
	pub ready_ms: Option<Timing>,
}

/// When an outcome last went one way: when it started, where and how.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Last {
	pub at: String,
	pub screen: String,
	pub ending: String,
}

/// A process family's last outcome that worked and last that failed, over
/// everything home holds, not only the window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProcessLast {
	pub process: String,
	pub worked: Option<Last>,
	pub failed: Option<Last>,
}

/// One affordance on one surface over the window.
///
/// How often each event was written, and when it was last taken over
/// everything home holds (paulgsc/some-ui ENG11, #2544).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AffordanceUse {
	pub pipeline: String,
	pub affordance: String,
	pub surface: String,
	pub offered: i64,
	pub taken: i64,
	pub passed_over: i64,
	pub left: i64,
	pub said: i64,
	pub last_taken: Option<String>,
}

/// What `GET /app-events/summary` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Summary {
	/// The window's start, as ISO-8601 UTC to the millisecond.
	pub since: String,
	/// Per family, the events home holds that started in the window.
	pub families: Vec<FamilyCount>,
	/// Per process family and screen, in that order.
	pub outcomes: Vec<ScreenOutcomes>,
	/// Per process family, in order.
	pub last: Vec<ProcessLast>,
	/// The window held more than [`WINDOW_READ`] outcomes, so `outcomes`
	/// counts only the newest of them.
	pub partial: bool,
	/// Per pipeline, affordance and surface, in that order: every one with
	/// an engagement event in the window, or ever taken.
	pub engagement: Vec<AffordanceUse>,
}

/// One process and screen's outcomes as a summary reads them.
#[derive(Default)]
struct Tally {
	worked: usize,
	failed: usize,
	stopped: usize,
	ready: Vec<u32>,
}

fn median_of(mut values: Vec<u32>) -> Option<Timing> {
	values.sort_unstable();
	let of = values.len();
	let median = *values.get(of / 2)?;
	let max = *values.last()?;
	Some(Timing { median, max, of })
}

/// `subject`'s events since `since` (an instant, as [`instant`] writes it),
/// summarised per family, and its outcomes per process and screen, with each
/// process's last outcome that worked and that failed.
///
/// Reads at most [`WINDOW_READ`] outcomes of the window, newest first, and
/// one grouped row per process and way it went for the lasts.
///
/// # Errors
/// [`EventError::Unreadable`] for a stored body that does not read back, and
/// [`EventError::Storage`] for any `sqlx` failure.
pub async fn summary(conn: &mut SqliteConnection, subject: &str, since: &str) -> Result<Summary, EventError> {
	let families = sqlx::query!(
		r#"SELECT family AS "family!: String", COUNT(*) AS "events!: i64"
		FROM app_event WHERE subject_id = ?1 AND at >= ?2
		GROUP BY family ORDER BY family"#,
		subject,
		since
	)
	.fetch_all(&mut *conn)
	.await?
	.into_iter()
	.map(|row| FamilyCount {
		family: row.family,
		events: row.events,
	})
	.collect();

	let bodies = sqlx::query_scalar!(
		"SELECT body FROM app_event WHERE subject_id = ?1 AND family = 'outcome' AND at >= ?2 ORDER BY at DESC, rowid DESC LIMIT ?3",
		subject,
		since,
		WINDOW_READ + 1
	)
	.fetch_all(&mut *conn)
	.await?;
	let partial = i64::try_from(bodies.len()).unwrap_or(i64::MAX) > WINDOW_READ;
	let mut by_screen: BTreeMap<(String, String), Tally> = BTreeMap::new();
	for body in bodies.iter().take(usize::try_from(WINDOW_READ).unwrap_or(usize::MAX)) {
		let Event::Outcome(outcome) = read_body(body)? else {
			return Err(EventError::Unreadable);
		};
		let entry = by_screen.entry((outcome.process, outcome.screen)).or_default();
		match outcome.went {
			Went::Worked => entry.worked += 1,
			Went::Failed => entry.failed += 1,
			Went::Stopped => entry.stopped += 1,
		}
		entry.ready.extend(outcome.ready_ms);
	}
	let outcomes = by_screen
		.into_iter()
		.map(|((process, screen), tally)| ScreenOutcomes {
			process,
			screen,
			worked: tally.worked,
			failed: tally.failed,
			stopped: tally.stopped,
			ready_ms: median_of(tally.ready),
		})
		.collect();

	// The newest outcome per process and way it went: SQLite's bare columns
	// beside MAX() come from the row that holds the maximum.
	let lasts = sqlx::query!(
		r#"SELECT body AS "body!: String", MAX(at) AS "at!: String"
		FROM app_event WHERE subject_id = ?1 AND family = 'outcome'
		GROUP BY json_extract(body, '$.process'), json_extract(body, '$.went')"#,
		subject
	)
	.fetch_all(&mut *conn)
	.await?;
	let mut last: BTreeMap<String, ProcessLast> = BTreeMap::new();
	for row in lasts {
		let Event::Outcome(outcome) = read_body(&row.body)? else {
			return Err(EventError::Unreadable);
		};
		let entry = last.entry(outcome.process.clone()).or_insert_with(|| ProcessLast {
			process: outcome.process.clone(),
			worked: None,
			failed: None,
		});
		let seen = Some(Last {
			at: row.at,
			screen: outcome.screen,
			ending: outcome.ending,
		});
		match outcome.went {
			Went::Worked => entry.worked = seen,
			Went::Failed => entry.failed = seen,
			Went::Stopped => {}
		}
	}

	Ok(Summary {
		since: since.to_owned(),
		families,
		outcomes,
		last: last.into_values().collect(),
		partial,
		engagement: engagement(conn, subject, since).await?,
	})
}

/// `subject`'s engagement events counted per affordance and kind over the
/// window, with each affordance's last `taken` over everything home holds:
/// one grouped read, no body read into memory.
async fn engagement(conn: &mut SqliteConnection, subject: &str, since: &str) -> Result<Vec<AffordanceUse>, EventError> {
	Ok(
		sqlx::query_as!(
			AffordanceUse,
			r#"SELECT pipeline AS "pipeline!: String", affordance AS "affordance!: String", surface AS "surface!: String",
			COALESCE(SUM(at >= ?2 AND kind = 'offered'), 0) AS "offered!: i64",
			COALESCE(SUM(at >= ?2 AND kind = 'taken'), 0) AS "taken!: i64",
			COALESCE(SUM(at >= ?2 AND kind = 'passed-over'), 0) AS "passed_over!: i64",
			COALESCE(SUM(at >= ?2 AND kind = 'left'), 0) AS "left!: i64",
			COALESCE(SUM(at >= ?2 AND kind = 'said'), 0) AS "said!: i64",
			MAX(CASE WHEN kind = 'taken' THEN at END) AS "last_taken: String"
		FROM (SELECT at, json_extract(body, '$.pipeline') AS pipeline, json_extract(body, '$.affordance') AS affordance,
				json_extract(body, '$.surface') AS surface, json_extract(body, '$.kind') AS kind
			FROM app_event WHERE subject_id = ?1 AND family = 'engagement')
		GROUP BY pipeline, affordance, surface
		HAVING SUM(at >= ?2) > 0 OR MAX(CASE WHEN kind = 'taken' THEN at END) IS NOT NULL
		ORDER BY pipeline, affordance, surface"#,
			subject,
			since
		)
		.fetch_all(&mut *conn)
		.await?,
	)
}

/// How many times one affordance was left at one of its places.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceLeft {
	pub place: String,
	pub left: i64,
}

/// `subject`'s `left` engagement events of `pipeline`'s `affordance`
/// between `from` and `to` (both included, ISO-8601 UTC to the
/// millisecond, as `at` is stored), counted per place, in place order.
///
/// It is how Rust Rounds' week record says how often each ticket was opened
/// (paulgsc/some-ui#2689, RR14) without a counter of its own: the phone's
/// `week-ticket` affordance is left at the ticket's place (`ticket-1` …)
/// each time its card is swiped away or the screen goes. One range read of
/// the subject's engagement events in the window, by
/// `app_event_by_family`; no body leaves but the place's name.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn left_by_place(conn: &mut SqliteConnection, subject: &str, pipeline: &str, affordance: &str, from: &str, to: &str) -> Result<Vec<PlaceLeft>, EventError> {
	Ok(
		sqlx::query_as!(
			PlaceLeft,
			r#"SELECT place AS "place!: String", COUNT(*) AS "left!: i64"
		FROM (SELECT json_extract(body, '$.place') AS place FROM app_event
			WHERE subject_id = ?1 AND family = 'engagement' AND at >= ?2 AND at <= ?3
			  AND json_extract(body, '$.kind') = 'left'
			  AND json_extract(body, '$.pipeline') = ?4 AND json_extract(body, '$.affordance') = ?5)
		WHERE place IS NOT NULL
		GROUP BY place ORDER BY place"#,
			subject,
			from,
			to,
			pipeline,
			affordance
		)
		.fetch_all(&mut *conn)
		.await?,
	)
}

#[cfg(test)]
mod tests {
	use super::{put_event, summary, Change, EventError, Last, Timing};
	use serde_json::{json, Value};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const PREVIOUS_MIGRATION: i64 = 20_261_103_000_100;
	const NOW: &str = "2026-11-03T23:00:00.000Z";
	const A: &str = "subject-a";
	const B: &str = "subject-b";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn outcome(id: &str, at: &str, screen: &str, ending: &str, went: &str, ready: Option<u32>) -> Value {
		json!({
			"family": "outcome", "id": id, "at": at, "build": "42a72b958",
			"process": "phone-recording", "screen": screen, "ending": ending, "went": went,
			"readyMs": ready, "durationMs": 4_000, "peak": 0.4
		})
	}

	async fn put(pool: &SqlitePool, subject: &str, value: &Value) -> Result<Change, EventError> {
		put_event(
			&mut pool.acquire().await.unwrap(),
			subject,
			value["id"].as_str().unwrap(),
			value.to_string().as_bytes(),
			NOW,
		)
		.await
		.map(|(change, _)| change)
	}

	async fn count(pool: &SqlitePool) -> i64 {
		sqlx::query_scalar("SELECT COUNT(*) FROM app_event").fetch_one(pool).await.unwrap()
	}

	#[tokio::test]
	async fn the_migration_round_trips_and_refuses_to_drop_a_record() {
		let pool = pool().await;
		let tables = || sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'app_event'");
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 1);
		MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 0, "down drops it while it is empty");
		MIGRATOR.run(&pool).await.unwrap();
		assert_eq!(tables().fetch_one(&pool).await.unwrap(), 1, "up recreates it");

		put(&pool, A, &outcome("e1", "2026-11-03T09:00:00.000Z", "jobs.pass", "kept", "worked", Some(300)))
			.await
			.unwrap();
		assert!(MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.is_err(), "a held event refuses the rollback");
		assert_eq!(count(&pool).await, 1);
	}

	/// The columns the migration describes and no others: a new one is a
	/// decision about what the server learns from the record.
	#[tokio::test]
	async fn the_table_holds_exactly_its_described_columns() {
		let pool = pool().await;
		let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('app_event') ORDER BY cid")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(columns, ["subject_id", "event_id", "family", "at", "body", "written_at"]);
	}

	#[tokio::test]
	async fn an_event_is_stored_once_and_never_replaced() {
		let pool = pool().await;
		let sent = outcome("e1", "2026-11-03T09:00:00.000Z", "jobs.pass", "kept", "worked", Some(300));
		assert_eq!(put(&pool, A, &sent).await.unwrap(), Change::Added);
		assert_eq!(put(&pool, A, &sent).await.unwrap(), Change::Unchanged, "a retry stores nothing");
		let mut other = sent.clone();
		other["went"] = json!("failed");
		assert!(matches!(put(&pool, A, &other).await, Err(EventError::Rewritten)));
		assert_eq!(put(&pool, B, &other).await.unwrap(), Change::Added, "another subject's id is its own");
		assert_eq!(count(&pool).await, 2);
		let update = sqlx::query("UPDATE app_event SET body = '{}'").execute(&pool).await;
		assert!(update.is_err(), "the table refuses an UPDATE");
	}

	#[tokio::test]
	async fn a_refused_write_writes_nothing() {
		let pool = pool().await;
		let mut wrong = outcome("e1", "2026-11-03T09:00:00.000Z", "jobs.pass", "kept", "worked", None);
		wrong["screen"] = json!("Jobs, the pass screen");
		assert!(matches!(put(&pool, A, &wrong).await, Err(EventError::Invalid(_))));
		assert_eq!(count(&pool).await, 0);
	}

	#[tokio::test]
	async fn the_summary_counts_per_process_and_screen_and_keeps_the_lasts() {
		let pool = pool().await;
		for event in [
			outcome("e1", "2026-10-01T09:00:00.000Z", "jobs.pass", "kept", "worked", Some(100)),
			outcome("e2", "2026-11-02T09:00:00.000Z", "jobs.pass", "kept", "worked", Some(300)),
			outcome("e3", "2026-11-02T10:00:00.000Z", "jobs.pass", "kept", "worked", Some(200)),
			outcome("e4", "2026-11-02T11:00:00.000Z", "jobs.pass", "blocked-denied", "failed", None),
			outcome("e5", "2026-11-02T12:00:00.000Z", "soundbites", "short", "stopped", Some(900)),
		] {
			put(&pool, A, &event).await.unwrap();
		}
		put(&pool, B, &outcome("e9", "2026-11-02T13:00:00.000Z", "jobs.pass", "failed", "failed", None))
			.await
			.unwrap();
		let held = summary(&mut pool.acquire().await.unwrap(), A, "2026-11-01T00:00:00.000Z").await.unwrap();
		assert_eq!(serde_json::to_value(&held.families).unwrap(), json!([{ "family": "outcome", "events": 4 }]));
		assert_eq!(
			serde_json::to_value(&held.outcomes).unwrap(),
			json!([
				{ "process": "phone-recording", "screen": "jobs.pass", "worked": 2, "failed": 1, "stopped": 0, "readyMs": { "median": 300, "max": 300, "of": 2 } },
				{ "process": "phone-recording", "screen": "soundbites", "worked": 0, "failed": 0, "stopped": 1, "readyMs": { "median": 900, "max": 900, "of": 1 } }
			])
		);
		assert!(!held.partial);
		assert_eq!(held.last.len(), 1);
		assert_eq!(
			held.last[0].worked,
			Some(Last {
				at: "2026-11-02T10:00:00.000Z".to_owned(),
				screen: "jobs.pass".to_owned(),
				ending: "kept".to_owned()
			})
		);
		assert_eq!(held.last[0].failed.as_ref().map(|last| last.ending.as_str()), Some("blocked-denied"));

		let empty = summary(&mut pool.acquire().await.unwrap(), "nobody", "2026-11-01T00:00:00.000Z").await.unwrap();
		assert!(empty.families.is_empty() && empty.outcomes.is_empty() && empty.last.is_empty());
		assert_eq!(super::median_of(vec![5, 1, 3, 9]), Some(Timing { median: 5, max: 9, of: 4 }));
	}

	fn engaged(id: &str, at: &str, affordance: &str, kind: &str) -> Value {
		let mut event = json!({
			"family": "engagement", "id": id, "at": at, "build": null,
			"kind": kind, "pipeline": "topik", "affordance": affordance, "surface": "apk"
		});
		match kind {
			"passed-over" => {
				event["offeredAt"] = json!(at);
				event["ending"] = json!("took-another");
			}
			"left" => {
				event["startedAt"] = json!(at);
				event["place"] = json!("quiz");
			}
			"said" => event["answer"] = json!("too-easy"),
			_ => {}
		}
		event
	}

	#[tokio::test]
	async fn the_summary_counts_engagement_per_affordance_and_keeps_the_last_taken() {
		let pool = pool().await;
		for event in [
			engaged("g1", "2026-10-01T09:00:00.000Z", "routine-lens", "taken"),
			engaged("g2", "2026-11-02T09:00:00.000Z", "routine-lens", "offered"),
			engaged("g3", "2026-11-02T09:00:00.000Z", "routine-read-aloud", "offered"),
			engaged("g4", "2026-11-02T09:01:00.000Z", "routine-read-aloud", "taken"),
			engaged("g5", "2026-11-02T09:01:00.000Z", "routine-lens", "passed-over"),
			engaged("g6", "2026-11-02T09:02:00.000Z", "routine-read-aloud", "left"),
			engaged("g7", "2026-11-02T09:03:00.000Z", "sentence-feedback", "said"),
			{
				let mut web = engaged("g8", "2026-11-02T09:04:00.000Z", "conversation", "taken");
				web["surface"] = json!("web");
				web
			},
			outcome("e1", "2026-11-02T09:02:00.000Z", "jobs.pass", "kept", "worked", None),
		] {
			put(&pool, A, &event).await.unwrap();
		}
		put(&pool, B, &engaged("g9", "2026-11-02T09:00:00.000Z", "routine-drama", "taken")).await.unwrap();
		let held = summary(&mut pool.acquire().await.unwrap(), A, "2026-11-01T00:00:00.000Z").await.unwrap();
		assert_eq!(
			serde_json::to_value(&held.families).unwrap(),
			json!([{ "family": "engagement", "events": 7 }, { "family": "outcome", "events": 1 }])
		);
		assert_eq!(held.outcomes.len(), 1, "engagement is not read as an outcome");
		assert_eq!(
			serde_json::to_value(&held.engagement).unwrap(),
			json!([
				{ "pipeline": "topik", "affordance": "conversation", "surface": "web", "offered": 0, "taken": 1, "passedOver": 0, "left": 0, "said": 0, "lastTaken": "2026-11-02T09:04:00.000Z" },
				{ "pipeline": "topik", "affordance": "routine-lens", "surface": "apk", "offered": 1, "taken": 0, "passedOver": 1, "left": 0, "said": 0, "lastTaken": "2026-10-01T09:00:00.000Z" },
				{ "pipeline": "topik", "affordance": "routine-read-aloud", "surface": "apk", "offered": 1, "taken": 1, "passedOver": 0, "left": 1, "said": 0, "lastTaken": "2026-11-02T09:01:00.000Z" },
				{ "pipeline": "topik", "affordance": "sentence-feedback", "surface": "apk", "offered": 0, "taken": 0, "passedOver": 0, "left": 0, "said": 1, "lastTaken": null }
			]),
			"the lens was last taken before the window, and B's drama is B's"
		);
	}

	/// A ticket's opens are its place's `left` events in the window, the
	/// subject's own, of that pipeline and affordance only.
	#[tokio::test]
	async fn left_events_are_counted_per_place_in_the_window() {
		let pool = pool().await;
		let left = |id: &str, at: &str, affordance: &str, place: &str| {
			let mut event = engaged(id, at, affordance, "left");
			event["pipeline"] = json!("rust-rounds");
			event["place"] = json!(place);
			event
		};
		for event in [
			left("o1", "2026-11-10T09:00:00.000Z", "week-ticket", "ticket-1"),
			left("o2", "2026-11-11T09:00:00.000Z", "week-ticket", "ticket-1"),
			left("o3", "2026-11-11T09:05:00.000Z", "week-ticket", "ticket-3"),
			left("o4", "2026-11-16T09:00:00.000Z", "week-ticket", "ticket-2"),
			left("o5", "2026-11-11T09:00:00.000Z", "card", "ticket-2"),
			engaged("o6", "2026-11-11T09:00:00.000Z", "week-ticket", "taken"),
		] {
			put(&pool, A, &event).await.unwrap();
		}
		put(&pool, B, &left("o7", "2026-11-11T09:00:00.000Z", "week-ticket", "ticket-2")).await.unwrap();
		let counted: Vec<(String, i64)> = super::left_by_place(
			&mut pool.acquire().await.unwrap(),
			A,
			"rust-rounds",
			"week-ticket",
			"2026-11-09T08:00:00.000Z",
			"2026-11-16T07:59:00.000Z",
		)
		.await
		.unwrap()
		.into_iter()
		.map(|place| (place.place, place.left))
		.collect();
		assert_eq!(counted, [(String::from("ticket-1"), 2), (String::from("ticket-3"), 1)]);
	}

	/// The four weeks count only their own events, a week before anything
	/// was held is not measured, and nothing after the asked week's end
	/// changes the answer.
	#[tokio::test]
	async fn the_weekly_read_counts_four_weeks_and_ignores_what_came_after() {
		let pool = pool().await;
		for event in [
			engaged("w1", "2026-08-12T09:00:00.000Z", "routine-lens", "taken"),
			engaged("w2", "2026-11-03T09:00:00.000Z", "routine-lens", "offered"),
			engaged("w3", "2026-11-09T07:59:59.999Z", "routine-lens", "offered"),
			engaged("w4", "2026-11-09T08:00:00.000Z", "routine-lens", "offered"),
			engaged("w5", "2026-11-09T08:00:01.000Z", "routine-lens", "taken"),
			engaged("w6", "2026-11-16T07:59:59.999Z", "routine-read-aloud", "passed-over"),
			engaged("w7", "2026-11-12T09:00:00.000Z", "routine-read-aloud", "left"),
			engaged("w8", "2026-11-12T09:00:00.000Z", "sentence-feedback", "said"),
			engaged("w9", "2026-11-16T08:00:00.000Z", "routine-drama", "taken"),
			outcome("e1", "2026-11-10T09:00:00.000Z", "jobs.pass", "kept", "worked", None),
		] {
			put(&pool, A, &event).await.unwrap();
		}
		put(&pool, B, &engaged("w10", "2026-11-10T09:00:00.000Z", "routine-drama", "taken")).await.unwrap();
		let week = super::IsoWeek::parse("2026-W46").unwrap();
		let read = |now: &'static str| {
			let pool = pool.clone();
			async move { super::weekly(&mut pool.acquire().await.unwrap(), A, week, now).await.unwrap() }
		};
		let held = read("2026-11-13T12:00:00.000Z").await;
		assert_eq!(
			serde_json::to_value(&held).unwrap(),
			json!({
				"week": "2026-W46",
				"weeks": [
					{ "role": "this", "week": "2026-W46", "from": "2026-11-09T08:00:00.000Z", "to": "2026-11-16T08:00:00.000Z",
						"complete": false, "measured": true, "affordances": [
						{ "pipeline": "topik", "affordance": "routine-lens", "surface": "apk", "offered": 1, "taken": 1, "passedOver": 0, "left": 0 },
						{ "pipeline": "topik", "affordance": "routine-read-aloud", "surface": "apk", "offered": 0, "taken": 0, "passedOver": 1, "left": 1 }
					] },
					{ "role": "last", "week": "2026-W45", "from": "2026-11-02T08:00:00.000Z", "to": "2026-11-09T08:00:00.000Z",
						"complete": true, "measured": true, "affordances": [
						{ "pipeline": "topik", "affordance": "routine-lens", "surface": "apk", "offered": 2, "taken": 0, "passedOver": 0, "left": 0 }
					] },
					{ "role": "quarterAgo", "week": "2026-W33", "from": "2026-08-10T07:00:00.000Z", "to": "2026-08-17T07:00:00.000Z",
						"complete": true, "measured": true, "affordances": [
						{ "pipeline": "topik", "affordance": "routine-lens", "surface": "apk", "offered": 0, "taken": 1, "passedOver": 0, "left": 0 }
					] },
					{ "role": "yearAgo", "week": "2025-W46", "from": "2025-11-10T08:00:00.000Z", "to": "2025-11-17T08:00:00.000Z",
						"complete": true, "measured": false, "affordances": [] }
				],
				"seen": [
					{ "pipeline": "topik", "affordance": "routine-lens", "surface": "apk", "firstDay": "2026-08-12", "lastTakenDay": "2026-11-09" },
					{ "pipeline": "topik", "affordance": "routine-read-aloud", "surface": "apk", "firstDay": "2026-11-12", "lastTakenDay": null },
					{ "pipeline": "topik", "affordance": "sentence-feedback", "surface": "apk", "firstDay": "2026-11-12", "lastTakenDay": null }
				]
			}),
			"Sunday evening Pacific is last week's, the said answer is seen but never counted, w9 is the next week's, and B's drama is B's"
		);
		let later = read("2027-01-01T00:00:00.000Z").await;
		assert!(later.weeks[0].complete);
		assert_eq!(later.seen, held.seen, "a later ask of the same week reads the same records");
		assert_eq!(later.weeks[0].affordances, held.weeks[0].affordances);
	}

	#[tokio::test]
	async fn the_record_leaves_with_the_account() {
		let pool = pool().await;
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?2)")
			.bind(A)
			.bind(A.as_bytes())
			.execute(&pool)
			.await
			.unwrap();
		put(&pool, A, &outcome("e1", "2026-11-03T09:00:00.000Z", "jobs.pass", "kept", "worked", None))
			.await
			.unwrap();
		put(&pool, B, &outcome("e1", "2026-11-03T09:00:00.000Z", "jobs.pass", "kept", "worked", None))
			.await
			.unwrap();
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		assert_eq!(count(&pool).await, 1, "only B's event stays");
	}
}
