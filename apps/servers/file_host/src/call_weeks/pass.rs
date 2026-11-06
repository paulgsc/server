//! One pass of the call's weeks: read `paulgsc/testing`'s `call/index.json`,
//! then fetch and keep each week it lists that home does not hold.
//!
//! - **Only what is missing.** A pass is one request for the index, plus one
//!   per listed week whose first day home holds no week of. A week home holds
//!   is never fetched again.
//! - **Checked before kept.** The index must read as `some-ui.call-index/1`,
//!   each week a `YYYY-MM-DD` pair, first day no later than last. A week's
//!   file must read as `some-ui.call-week/1` for exactly the days the index
//!   names, and be at most [`MAX_BODY`] bytes. Its full shape is the call's
//!   own (`@some-ui/video-render`'s `CallWeekSchema`), held by the routine's
//!   `call-check` before the week merged; home keeps the bytes as served.
//! - **Counts and kinds, never bytes.** What went wrong is a kind
//!   ([`Pass::failures`]): no URL, body or week's words reaches a log.
//! - **No network in tests.** Fetching is `game_feed`'s [`Fetch`] port.

use crate::game_feed::pass::{Answer, Fetch};
use call_week_repo::{Kept, Week};
use chrono::{DateTime, Duration as Span, NaiveDate, SecondsFormat, Utc};
use serde_json::Value;
use sqlx::SqlitePool;
use std::time::Duration;

/// Where the weeks are: `paulgsc/testing`'s `call/` folder on `main`,
/// public, served without a token.
pub const SOURCE: &str = "https://raw.githubusercontent.com/paulgsc/testing/main/call/";

/// What `call/index.json` is.
pub const INDEX_SCHEMA: &str = "some-ui.call-index/1";

/// What a week's file is.
pub const WEEK_SCHEMA: &str = "some-ui.call-week/1";

/// The most bytes home takes for the index or a week: a week is about
/// 80 KB today, and the mailbox's own cap is the same.
pub const MAX_BODY: usize = 1_048_576;

/// What a pass did: the weeks it added, and each thing that went wrong, by
/// kind.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Pass {
	/// Weeks stored this pass.
	pub added: usize,
	/// Listed weeks home already held a week of that first day for.
	pub held: usize,
	/// Each failure, by kind: `timeout`, `connect`, `transport`, `refused`
	/// (a status other than 200), `too_big`, `unreadable`.
	pub failures: Vec<&'static str>,
}

/// When the next pass is due, and how many passes in a row went wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Schedule {
	next_at: DateTime<Utc>,
	failures: u32,
}

impl Schedule {
	/// Due at once, so a restart catches up without waiting an interval.
	#[must_use]
	pub const fn new(now: DateTime<Utc>) -> Self {
		Self { next_at: now, failures: 0 }
	}

	/// Whether a pass is due at `now`.
	#[must_use]
	pub fn due(&self, now: DateTime<Utc>) -> bool {
		self.next_at <= now
	}

	/// After a pass at `now`: the next one an `every` later when it went
	/// well; when it did not, `retry` doubled per failure in a row, to at
	/// most `every`, so a failing source is asked no more than a working one.
	pub fn after(&mut self, now: DateTime<Utc>, failed: bool, every: Duration, retry: Duration) {
		if failed {
			self.failures = self.failures.saturating_add(1);
			let factor = 2_u32.saturating_pow(self.failures.saturating_sub(1).min(16));
			self.next_at = now + span(retry.saturating_mul(factor).min(every));
		} else {
			self.failures = 0;
			self.next_at = now + span(every);
		}
	}
}

fn span(duration: Duration) -> Span {
	Span::from_std(duration).unwrap_or(Span::MAX)
}

/// A week the index lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
	pub from: String,
	pub to: String,
}

/// `text` as a calendar day, `YYYY-MM-DD`.
fn is_day(text: &str) -> bool {
	text.len() == 10 && NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok()
}

/// One listed week of an index, when it reads as one.
fn listed(entry: &Value) -> Option<Listed> {
	let entry = entry.as_object()?;
	let (from, to) = (entry.get("from")?.as_str()?, entry.get("to")?.as_str()?);
	(entry.len() == 2 && is_day(from) && is_day(to) && from <= to).then(|| Listed {
		from: from.to_owned(),
		to: to.to_owned(),
	})
}

/// The weeks `call/index.json` lists, or `None` when it does not read as
/// `some-ui.call-index/1`.
#[must_use]
pub fn read_index(body: &[u8]) -> Option<Vec<Listed>> {
	let index: Value = serde_json::from_slice(body).ok()?;
	if index.get("schema")?.as_str()? != INDEX_SCHEMA {
		return None;
	}
	index.get("weeks")?.as_array()?.iter().map(listed).collect()
}

/// Whether a week's file reads as `some-ui.call-week/1` for exactly the days
/// the index names.
#[must_use]
pub fn is_week(body: &[u8], listed: &Listed) -> bool {
	let Ok(week) = serde_json::from_slice::<Value>(body) else {
		return false;
	};
	let days = week.get("week");
	week.get("schema").and_then(Value::as_str) == Some(WEEK_SCHEMA)
		&& days.and_then(|days| days.get("from")).and_then(Value::as_str) == Some(listed.from.as_str())
		&& days.and_then(|days| days.get("to")).and_then(Value::as_str) == Some(listed.to.as_str())
}

/// Fetches `url`: its body when it answered 200 with at most [`MAX_BODY`]
/// bytes, else the failure's kind, pushed onto `pass`.
async fn fetched<F: Fetch>(fetch: &F, url: &str, pass: &mut Pass) -> Option<Vec<u8>> {
	match fetch.get(url).await {
		Ok(Answer { status: 200, body }) if body.len() <= MAX_BODY => Some(body),
		Ok(Answer { status: 200, .. }) => {
			pass.failures.push("too_big");
			None
		}
		Ok(_) => {
			pass.failures.push("refused");
			None
		}
		Err(no_answer) => {
			pass.failures.push(no_answer.0);
			None
		}
	}
}

/// One pass: read the index, then fetch and keep each week it lists that
/// home holds no week of, oldest first. A week that fails is left for the
/// next pass; the others are still taken.
///
/// # Errors
/// A storage failure, which ends the pass.
pub async fn run_once<F: Fetch>(db: &SqlitePool, fetch: &F, clock: &(dyn Fn() -> DateTime<Utc> + Sync)) -> Result<Pass, sqlx::Error> {
	let mut pass = Pass::default();
	let Some(body) = fetched(fetch, &(String::from(SOURCE) + "index.json"), &mut pass).await else {
		return Ok(pass);
	};
	let Some(mut weeks) = read_index(&body) else {
		pass.failures.push("unreadable");
		return Ok(pass);
	};
	let held = {
		let mut conn = db.acquire().await?;
		call_week_repo::held_days(&mut conn).await?
	};
	weeks.sort_by(|a, b| a.from.cmp(&b.from));
	weeks.dedup_by(|a, b| a.from == b.from);
	for listed in weeks {
		if held.binary_search(&listed.from).is_ok() {
			pass.held += 1;
			continue;
		}
		// `from` is a checked calendar day, so the URL names one file.
		let Some(body) = fetched(fetch, &(String::from(SOURCE) + &listed.from + ".json"), &mut pass).await else {
			continue;
		};
		if !is_week(&body, &listed) {
			pass.failures.push("unreadable");
			continue;
		}
		let taken_at = clock().to_rfc3339_opts(SecondsFormat::Millis, true);
		let week = Week {
			from: &listed.from,
			to: &listed.to,
			body: &body,
			taken_at: &taken_at,
		};
		let mut conn = db.acquire().await?;
		match call_week_repo::keep(&mut conn, &week).await? {
			Kept::Added => pass.added += 1,
			Kept::Held => pass.held += 1,
		}
	}
	Ok(pass)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::game_feed::pass::NoAnswer;
	use sqlx::sqlite::SqlitePoolOptions;
	use std::collections::HashMap;
	use std::sync::Mutex;

	/// Answers each URL from a table, and records every request.
	#[derive(Default)]
	struct Fake {
		answers: Mutex<HashMap<String, Result<Answer, NoAnswer>>>,
		asked: Mutex<Vec<String>>,
	}

	impl Fake {
		fn answer(&self, name: &str, answer: Result<Answer, NoAnswer>) {
			self.answers.lock().unwrap().insert(String::from(SOURCE) + name, answer);
		}

		fn ok(&self, name: &str, body: &[u8]) {
			self.answer(name, Ok(Answer { status: 200, body: body.to_vec() }));
		}

		fn asked(&self) -> Vec<String> {
			let asked = std::mem::take(&mut *self.asked.lock().unwrap());
			asked.into_iter().map(|url| url.trim_start_matches(SOURCE).to_owned()).collect()
		}
	}

	impl Fetch for Fake {
		async fn get(&self, url: &str) -> Result<Answer, NoAnswer> {
			self.asked.lock().unwrap().push(url.to_owned());
			self.answers.lock().unwrap().get(url).cloned().unwrap_or(Err(NoAnswer("connect")))
		}
	}

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		crate::schema::MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn clock() -> DateTime<Utc> {
		DateTime::parse_from_rfc3339("2026-11-06T09:00:00.000Z").unwrap().with_timezone(&Utc)
	}

	fn index(weeks: &[(&str, &str)]) -> Vec<u8> {
		let weeks: Vec<Value> = weeks.iter().map(|(from, to)| serde_json::json!({ "from": from, "to": to })).collect();
		serde_json::json!({ "schema": INDEX_SCHEMA, "keep": 8, "weeks": weeks }).to_string().into_bytes()
	}

	fn week(from: &str, to: &str) -> Vec<u8> {
		serde_json::json!({ "schema": WEEK_SCHEMA, "week": { "from": from, "to": to }, "open": { "said": "Hello." } })
			.to_string()
			.into_bytes()
	}

	#[tokio::test]
	async fn each_listed_week_home_lacks_is_fetched_once_and_kept_as_served() {
		let pool = pool().await;
		let fake = Fake::default();
		fake.ok("index.json", &index(&[("2026-11-06", "2026-11-12"), ("2026-10-30", "2026-11-05")]));
		let (older, newer) = (week("2026-10-30", "2026-11-05"), week("2026-11-06", "2026-11-12"));
		fake.ok("2026-10-30.json", &older);
		fake.ok("2026-11-06.json", &newer);

		let first = run_once(&pool, &fake, &clock).await.unwrap();
		assert_eq!((first.added, first.held, first.failures.len()), (2, 0, 0));
		assert_eq!(fake.asked(), ["index.json", "2026-10-30.json", "2026-11-06.json"], "oldest first");

		let again = run_once(&pool, &fake, &clock).await.unwrap();
		assert_eq!((again.added, again.held), (0, 2));
		assert_eq!(fake.asked(), ["index.json"], "a week home holds is never fetched again");

		// The repository lets the older week go: home still holds it.
		fake.ok("index.json", &index(&[("2026-11-06", "2026-11-12")]));
		run_once(&pool, &fake, &clock).await.unwrap();
		let mut conn = pool.acquire().await.unwrap();
		let held = call_week_repo::held(&mut conn).await.unwrap();
		assert_eq!(held.len(), 2);
		assert_eq!(call_week_repo::body(&mut conn, "2026-10-30").await.unwrap(), Some(older));
		assert_eq!((held[0].from.as_str(), held[0].to.as_str()), ("2026-11-06", "2026-11-12"));
	}

	#[tokio::test]
	async fn what_does_not_check_out_is_counted_by_kind_and_left_for_the_next_pass() {
		let pool = pool().await;
		let fake = Fake::default();
		assert_eq!(run_once(&pool, &fake, &clock).await.unwrap().failures, ["connect"]);
		fake.answer(
			"index.json",
			Ok(Answer {
				status: 429,
				body: b"slow down".to_vec(),
			}),
		);
		assert_eq!(run_once(&pool, &fake, &clock).await.unwrap().failures, ["refused"]);
		fake.ok("index.json", b"<html>busy</html>");
		assert_eq!(run_once(&pool, &fake, &clock).await.unwrap().failures, ["unreadable"]);
		fake.ok("index.json", &index(&[("2026-10-30", "2026-11-05"), ("../secrets", "2026-11-05")]));
		assert_eq!(run_once(&pool, &fake, &clock).await.unwrap().failures, ["unreadable"], "a week that is not a day");
		fake.asked();

		let listed = [
			("2026-11-13", "2026-11-19"),
			("2026-11-06", "2026-11-12"),
			("2026-10-30", "2026-11-05"),
			("2026-10-23", "2026-10-29"),
		];
		fake.ok("index.json", &index(&listed));
		fake.ok("2026-10-23.json", &vec![b' '; MAX_BODY + 1]);
		fake.ok("2026-10-30.json", &week("2026-10-30", "2026-11-04"));
		fake.answer("2026-11-06.json", Err(NoAnswer("timeout")));
		fake.ok("2026-11-13.json", &week("2026-11-13", "2026-11-19"));
		let pass = run_once(&pool, &fake, &clock).await.unwrap();
		assert_eq!(pass.failures, ["too_big", "unreadable", "timeout"]);
		assert_eq!(pass.added, 1, "the good week is still taken");

		fake.ok("2026-11-06.json", &week("2026-11-06", "2026-11-12"));
		fake.asked();
		let next = run_once(&pool, &fake, &clock).await.unwrap();
		assert_eq!(next.added, 1);
		assert_eq!(
			fake.asked(),
			["index.json", "2026-10-23.json", "2026-10-30.json", "2026-11-06.json"],
			"each failed week is asked again"
		);
	}

	#[test]
	fn a_failing_source_is_retried_sooner_but_never_more_often_than_it_is_read() {
		let (every, retry) = (Duration::from_secs(3600), Duration::from_secs(300));
		let now = clock();
		let mut schedule = Schedule::new(now);
		assert!(schedule.due(now));
		let mut waits = Vec::new();
		for _ in 0..6 {
			schedule.after(now, true, every, retry);
			waits.push((schedule.next_at - now).num_seconds());
		}
		assert_eq!(waits, [300, 600, 1200, 2400, 3600, 3600]);
		schedule.after(now, false, every, retry);
		assert_eq!((schedule.next_at - now).num_seconds(), 3600);
		schedule.after(now, true, every, retry);
		assert_eq!((schedule.next_at - now).num_seconds(), 300, "a good pass resets the count");
		assert!(!schedule.due(now));
	}
}
