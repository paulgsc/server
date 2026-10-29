//! When each scheduled routine is due, and when its newest run started
//! (AS7, paulgsc/server#525; objective RR2 in paulgsc/some-ui's ledger).
//!
//! A routine that never starts leaves no record, so "nothing found" and "did
//! not run" look alike unless something expects the run. After each mailbox
//! pass, [`stamp`] sets two gauges per routine:
//!
//! - `routine_due_timestamp_seconds`: the latest scheduled start at or before
//!   now, by the routine's own time zone's clock (`ROUTINE_SCHEDULE`);
//! - `routine_last_run_timestamp_seconds`: the `startedAt` of the newest run
//!   record home holds for it, or 0 when it holds none.
//!
//! `infra/prometheus/rules/home.yml`'s `RoutineRunOverdue` compares them.
//! Only those two instants leave home: no record's body or name.

use chrono::{DateTime, Duration, LocalResult, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use sqlx::SqlitePool;

/// The routines home holds run records for, by the name their records carry
/// (`run-<name>-<startedAt>.json`, `"routine": "<name>"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Routine {
	/// paulgsc/some-ui `packages/job-desk/ROUTINE.md`.
	Jobs,
	/// paulgsc/some-ui `packages/ui/topik/ROUTINE.md`.
	Reflections,
}

impl Routine {
	#[must_use]
	pub const fn name(self) -> &'static str {
		match self {
			Self::Jobs => "jobs",
			Self::Reflections => "reflections",
		}
	}

	fn parse(name: &str) -> Option<Self> {
		match name {
			"jobs" => Some(Self::Jobs),
			"reflections" => Some(Self::Reflections),
			_ => None,
		}
	}
}

/// One routine's daily start, in its own zone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
	pub routine: Routine,
	pub at: NaiveTime,
	pub zone: Tz,
}

/// Parse `ROUTINE_SCHEDULE`: `<routine> <HH:MM> <IANA zone>`, separated by
/// `;`, each routine at most once. Empty means no routine is expected.
///
/// # Errors
/// The first entry that does not parse, by its text: a schedule that is
/// silently ignored is the failure this exists to catch.
pub fn parse_schedules(text: &str) -> Result<Vec<Schedule>, String> {
	let mut schedules: Vec<Schedule> = Vec::new();
	for entry in text.split(';').map(str::trim).filter(|entry| !entry.is_empty()) {
		let parts: Vec<&str> = entry.split_whitespace().collect();
		let [routine, at, zone] = parts.as_slice() else {
			return Err(String::from("\"") + entry + "\" is not \"<routine> <HH:MM> <zone>\"");
		};
		let routine = Routine::parse(routine).ok_or_else(|| String::from("\"") + routine + "\" is not jobs or reflections")?;
		let at = NaiveTime::parse_from_str(at, "%H:%M").map_err(|_| String::from("\"") + at + "\" is not HH:MM")?;
		let zone = zone.parse::<Tz>().map_err(|_| String::from("\"") + zone + "\" is not a time zone")?;
		if schedules.iter().any(|schedule| schedule.routine == routine) {
			return Err(String::from(routine.name()) + " is scheduled twice");
		}
		schedules.push(Schedule { routine, at, zone });
	}
	Ok(schedules)
}

/// The latest scheduled start at or before `now`, by the routine's zone.
///
/// Today's, or else yesterday's. On a day the clock skips the time, the run
/// is due at the first instant after the gap; on a day it repeats it, at the
/// first of the two.
#[must_use]
pub fn last_due(schedule: &Schedule, now: DateTime<Utc>) -> DateTime<Utc> {
	let today = now.with_timezone(&schedule.zone).date_naive();
	due_on(schedule, today)
		.filter(|due| *due <= now)
		.or_else(|| due_on(schedule, today - Duration::days(1)))
		// No zone skips the same three hours two days running. Were one to,
		// a day back reads as due, so the run is expected, not excused.
		.unwrap_or(now - Duration::days(1))
}

fn due_on(schedule: &Schedule, day: NaiveDate) -> Option<DateTime<Utc>> {
	let local = day.and_time(schedule.at);
	match schedule.zone.from_local_datetime(&local) {
		LocalResult::Single(at) | LocalResult::Ambiguous(at, _) => Some(at.with_timezone(&Utc)),
		LocalResult::None => (1..=180)
			.find_map(|minutes| schedule.zone.from_local_datetime(&(local + Duration::minutes(minutes))).earliest())
			.map(|at| at.with_timezone(&Utc)),
	}
}

/// How many of a routine's newest records to read for the latest start: a
/// re-run can arrive after the run it follows.
const RECORDS_READ: i64 = 5;

/// The newest `startedAt` at or before `now` of `routine`'s runs.
///
/// Among the run records home holds for `subject`, or `None` when it holds
/// none. A start in the future (a clock or a typo) is not a run, or it would
/// excuse every due time until it passed.
///
/// # Errors
/// Any storage failure.
pub async fn last_started(db: &SqlitePool, subject: &str, routine: Routine, now: DateTime<Utc>) -> Result<Option<DateTime<Utc>>, mailbox_repo::MailboxError> {
	let mut conn = db.acquire().await.map_err(mailbox_repo::MailboxError::Storage)?;
	let records = match routine {
		Routine::Jobs => mailbox_repo::jobs_runs(&mut conn, subject, RECORDS_READ).await,
		Routine::Reflections => mailbox_repo::routine_runs(&mut conn, subject, RECORDS_READ).await,
	}?;
	Ok(records.iter().filter_map(|record| started_at(&record.body)).filter(|started| *started <= now).max())
}

fn started_at(body: &[u8]) -> Option<DateTime<Utc>> {
	let record: serde_json::Value = serde_json::from_slice(body).ok()?;
	let at = record.get("startedAt")?.as_str()?;
	DateTime::parse_from_rfc3339(at).ok().map(|at| at.with_timezone(&Utc))
}

/// Set both gauges for every scheduled routine. A read that fails leaves
/// that routine's last-run gauge as it was and says so in the log.
pub async fn stamp(db: &SqlitePool, subject: &str, schedules: &[Schedule], now: DateTime<Utc>) {
	for schedule in schedules {
		let routine = schedule.routine.name();
		crate::metrics::routine::record_due(routine, last_due(schedule, now));
		match last_started(db, subject, schedule.routine, now).await {
			Ok(started) => crate::metrics::routine::record_last_run(routine, started),
			Err(err) => tracing::error!(error = %err, routine, "could not read the routine's newest run"),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{last_due, last_started, parse_schedules, started_at, Routine, Schedule};
	use chrono::{DateTime, NaiveTime, Utc};
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?1)")
			.bind(A)
			.execute(&pool)
			.await
			.unwrap();
		pool
	}

	/// A run record the poller kept, as the routine names it.
	async fn run(pool: &SqlitePool, seq: i64, routine: &str, body: &str) {
		let name = String::from("run-") + routine + "-" + &seq.to_string() + ".json";
		sqlx::query("INSERT INTO mailbox_item (subject_id, folder, name, seq, sha256, size, body, pulled_at) VALUES (?1, 'run', ?2, ?3, 's', 0, ?4, 'now')")
			.bind(A)
			.bind(&name)
			.bind(seq)
			.bind(body.as_bytes())
			.execute(pool)
			.await
			.unwrap();
	}

	#[tokio::test]
	async fn each_routines_latest_start_is_read_from_its_own_records() {
		let pool = pool().await;
		let now = utc("2026-10-28T18:00:00Z");
		assert_eq!(last_started(&pool, A, Routine::Jobs, now).await.unwrap(), None);
		run(&pool, 1, "jobs", r#"{"startedAt":"2026-10-27T14:52:07Z"}"#).await;
		// A re-run of an earlier morning, arriving after a later one.
		run(&pool, 2, "jobs", r#"{"startedAt":"2026-10-28T14:52:07Z"}"#).await;
		run(&pool, 3, "jobs", r#"{"startedAt":"2026-10-26T15:10:00Z"}"#).await;
		run(&pool, 4, "reflections", r#"{"startedAt":"2026-10-28T16:00:03Z"}"#).await;
		run(&pool, 5, "reflections", "not json").await;
		// A start in the future is no run: it would excuse every due time.
		run(&pool, 6, "jobs", r#"{"startedAt":"2027-10-28T14:52:07Z"}"#).await;
		assert_eq!(last_started(&pool, A, Routine::Jobs, now).await.unwrap(), Some(utc("2026-10-28T14:52:07Z")));
		assert_eq!(last_started(&pool, A, Routine::Reflections, now).await.unwrap(), Some(utc("2026-10-28T16:00:03Z")));
	}

	fn utc(at: &str) -> DateTime<Utc> {
		DateTime::parse_from_rfc3339(at).unwrap().with_timezone(&Utc)
	}

	fn jobs() -> Schedule {
		parse_schedules("jobs 07:52 America/Los_Angeles").unwrap().remove(0)
	}

	#[test]
	fn the_default_schedule_parses() {
		let schedules = parse_schedules(crate::config::DEFAULT_ROUTINE_SCHEDULE).unwrap();
		assert_eq!(schedules.iter().map(|s| s.routine).collect::<Vec<_>>(), [Routine::Jobs, Routine::Reflections]);
		assert_eq!(schedules[1].at, NaiveTime::from_hms_opt(9, 0, 0).unwrap());
	}

	#[test]
	fn a_bad_entry_is_refused_by_its_text() {
		for (text, said) in [
			("jobs 07:52", "is not \"<routine> <HH:MM> <zone>\""),
			("chores 07:52 UTC", "\"chores\" is not jobs or reflections"),
			("jobs 7.52 UTC", "\"7.52\" is not HH:MM"),
			("jobs 07:52 Mars/Olympus", "\"Mars/Olympus\" is not a time zone"),
			("jobs 07:52 UTC; jobs 08:00 UTC", "jobs is scheduled twice"),
		] {
			assert!(parse_schedules(text).unwrap_err().contains(said), "{text}");
		}
		assert_eq!(parse_schedules(" ; ").unwrap(), []);
	}

	#[test]
	fn a_run_is_due_by_its_zones_clock_across_standard_and_daylight_time() {
		// 07:52 Pacific is 14:52Z in daylight time and 15:52Z in standard.
		assert_eq!(last_due(&jobs(), utc("2026-10-28T15:00:00Z")), utc("2026-10-28T14:52:00Z"));
		assert_eq!(last_due(&jobs(), utc("2026-10-28T14:51:00Z")), utc("2026-10-27T14:52:00Z"));
		assert_eq!(last_due(&jobs(), utc("2026-12-01T16:00:00Z")), utc("2026-12-01T15:52:00Z"));
	}

	#[test]
	fn a_start_the_clock_skips_is_due_just_after_the_gap() {
		let at_two = parse_schedules("jobs 02:30 America/Los_Angeles").unwrap().remove(0);
		// 2027-03-14: 02:00 Pacific jumps to 03:00, 10:00Z.
		assert_eq!(last_due(&at_two, utc("2027-03-14T12:00:00Z")), utc("2027-03-14T10:00:00Z"));
	}

	#[test]
	fn a_start_the_clock_repeats_is_due_at_the_first_of_the_two() {
		let at_one = parse_schedules("jobs 01:30 America/Los_Angeles").unwrap().remove(0);
		// 2026-11-01: 01:30 Pacific happens at 08:30Z (daylight) and 09:30Z.
		assert_eq!(last_due(&at_one, utc("2026-11-01T12:00:00Z")), utc("2026-11-01T08:30:00Z"));
	}

	#[test]
	fn a_record_says_when_it_started_or_nothing() {
		assert_eq!(started_at(br#"{"startedAt":"2026-10-26T14:52:07Z"}"#), Some(utc("2026-10-26T14:52:07Z")));
		for body in [&b"{}"[..], b"not json", br#"{"startedAt":"yesterday"}"#] {
			assert_eq!(started_at(body), None);
		}
	}
}
