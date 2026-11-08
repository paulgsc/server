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

use crate::game_feed::pass::{fetch_capped, Fetch};
use call_week_repo::{Kept, Week};
use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use serde_json::Value;
use sqlx::SqlitePool;
use std::collections::HashSet;
use std::path::Path;

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
	/// Weeks written to the render worker's folder this pass.
	pub exported: usize,
	/// Each failure, by kind: `timeout`, `connect`, `transport`, `refused`
	/// (a status other than 200), `too_big`, `unreadable`, and `export`
	/// for a week that could not be written to the folder.
	pub failures: Vec<&'static str>,
}

/// A week the index lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
	pub from: String,
	pub to: String,
}

/// Whether `text` is a calendar day written `YYYY-MM-DD`, and nothing else:
/// chrono's parser also takes a sign, a space or a one-digit field, so the
/// day must print back as `text`.
fn is_day(text: &str) -> bool {
	NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok_and(|day| day.format("%Y-%m-%d").to_string() == text)
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

/// Fetches `url` ([`fetch_capped`] at [`MAX_BODY`]), pushing a failure's
/// kind onto `pass`.
async fn fetched<F: Fetch>(fetch: &F, url: &str, pass: &mut Pass) -> Option<Vec<u8>> {
	fetch_capped(fetch, url, MAX_BODY).await.map_err(|kind| pass.failures.push(kind)).ok()
}

/// One pass: [`take_new`], then [`export`].
///
/// It takes each week the repository lists that home lacks, then writes each
/// week home holds that `folder` lacks, so the render worker sees every week,
/// a restart included.
///
/// # Errors
/// A storage failure, which ends the pass.
pub async fn run_once<F: Fetch>(db: &SqlitePool, fetch: &F, folder: &Path, clock: &(dyn Fn() -> DateTime<Utc> + Sync)) -> Result<Pass, sqlx::Error> {
	let mut pass = Pass::default();
	take_new(db, fetch, clock, &mut pass).await?;
	export(db, folder, &mut pass).await?;
	Ok(pass)
}

/// Writes each week home holds that `folder` has no file of, as
/// `<from>.json`, byte for byte, through a temporary name so the render
/// worker never reads half a file. A week already written is left alone:
/// home never rewrites a kept week, so neither does this.
///
/// A week whose video the owner let go from My data (`videos::let_go`) is
/// never written, and its file is removed if one is left, so the worker
/// never renders it again (CW8).
async fn export(db: &SqlitePool, folder: &Path, pass: &mut Pass) -> Result<(), sqlx::Error> {
	// Held for the whole pass, so a week let go while it runs is never
	// written back (`super::FOLDERS`).
	let _folders = super::FOLDERS.lock().await;
	let (held, let_go) = {
		let mut conn = db.acquire().await?;
		(call_week_repo::held(&mut conn).await?, call_week_repo::video::let_go(&mut conn).await?)
	};
	for from in &let_go {
		match tokio::fs::remove_file(folder.join(String::from(from) + ".json")).await {
			Err(error) if error.kind() != std::io::ErrorKind::NotFound => pass.failures.push("export"),
			_ => {}
		}
	}
	let missing: Vec<_> = held
		.into_iter()
		.filter(|week| !let_go.contains(&week.from) && !folder.join(String::from(&week.from) + ".json").exists())
		.collect();
	if missing.is_empty() {
		return Ok(());
	}
	if tokio::fs::create_dir_all(folder).await.is_err() {
		pass.failures.push("export");
		return Ok(());
	}
	for week in missing {
		let body = {
			let mut conn = db.acquire().await?;
			call_week_repo::body(&mut conn, &week.from).await?
		};
		let Some(body) = body else { continue };
		let file = folder.join(String::from(&week.from) + ".json");
		let partial = folder.join(String::from(".") + &week.from + ".json.partial");
		let written = tokio::fs::write(&partial, &body).await.is_ok() && tokio::fs::rename(&partial, &file).await.is_ok();
		if written {
			pass.exported += 1;
		} else {
			pass.failures.push("export");
		}
	}
	Ok(())
}

/// Reads the index, then fetches and keeps each week it lists that home
/// holds no week of, oldest first. A week that fails is left for the next
/// pass; the others are still taken.
async fn take_new<F: Fetch>(db: &SqlitePool, fetch: &F, clock: &(dyn Fn() -> DateTime<Utc> + Sync), pass: &mut Pass) -> Result<(), sqlx::Error> {
	let Some(body) = fetched(fetch, &(String::from(SOURCE) + "index.json"), pass).await else {
		return Ok(());
	};
	let Some(mut weeks) = read_index(&body) else {
		pass.failures.push("unreadable");
		return Ok(());
	};
	let held: HashSet<String> = {
		let mut conn = db.acquire().await?;
		call_week_repo::held(&mut conn).await?.into_iter().map(|week| week.from).collect()
	};
	weeks.sort_by(|a, b| a.from.cmp(&b.from));
	weeks.dedup_by(|a, b| a.from == b.from);
	for listed in weeks {
		if held.contains(&listed.from) {
			pass.held += 1;
			continue;
		}
		// `from` is a checked calendar day, so the URL names one file.
		let Some(body) = fetched(fetch, &(String::from(SOURCE) + &listed.from + ".json"), pass).await else {
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
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::game_feed::pass::{Answer, NoAnswer, Target};
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
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let fake = Fake::default();
		fake.ok("index.json", &index(&[("2026-11-06", "2026-11-12"), ("2026-10-30", "2026-11-05")]));
		let (older, newer) = (week("2026-10-30", "2026-11-05"), week("2026-11-06", "2026-11-12"));
		fake.ok("2026-10-30.json", &older);
		fake.ok("2026-11-06.json", &newer);

		let first = run_once(&pool, &fake, dir.path(), &clock).await.unwrap();
		assert_eq!((first.added, first.held, first.failures.len()), (2, 0, 0));
		assert_eq!(fake.asked(), ["index.json", "2026-10-30.json", "2026-11-06.json"], "oldest first");

		let again = run_once(&pool, &fake, dir.path(), &clock).await.unwrap();
		assert_eq!((again.added, again.held), (0, 2));
		assert_eq!(fake.asked(), ["index.json"], "a week home holds is never fetched again");
	}

	#[tokio::test]
	async fn what_does_not_check_out_is_counted_by_kind_and_left_for_the_next_pass() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let fake = Fake::default();
		assert_eq!(run_once(&pool, &fake, dir.path(), &clock).await.unwrap().failures, ["connect"]);
		fake.answer(
			"index.json",
			Ok(Answer {
				status: 429,
				body: b"slow down".to_vec(),
			}),
		);
		assert_eq!(run_once(&pool, &fake, dir.path(), &clock).await.unwrap().failures, ["refused"]);
		fake.ok("index.json", b"<html>busy</html>");
		assert_eq!(run_once(&pool, &fake, dir.path(), &clock).await.unwrap().failures, ["unreadable"]);
		for day in ["../secrets", "+2026-1-05", " 2026-1-05", "2026-02-30"] {
			fake.ok("index.json", &index(&[("2026-10-30", "2026-11-05"), (day, "2026-11-05")]));
			assert_eq!(run_once(&pool, &fake, dir.path(), &clock).await.unwrap().failures, ["unreadable"], "{day:?} is not a day");
		}
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
		let pass = run_once(&pool, &fake, dir.path(), &clock).await.unwrap();
		assert_eq!(pass.failures, ["too_big", "unreadable", "timeout"]);
		assert_eq!(pass.added, 1, "the good week is still taken");

		fake.ok("2026-11-06.json", &week("2026-11-06", "2026-11-12"));
		fake.asked();
		let next = run_once(&pool, &fake, dir.path(), &clock).await.unwrap();
		assert_eq!(next.added, 1);
		assert_eq!(
			fake.asked(),
			["index.json", "2026-10-23.json", "2026-10-30.json", "2026-11-06.json"],
			"each failed week is asked again"
		);
	}

	/// The render worker's folder gets each week home holds, byte for byte,
	/// once: a week already there is never rewritten, and a pass whose index
	/// can't be read still writes what home holds.
	#[tokio::test]
	async fn every_week_held_is_written_for_the_render_worker_once() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let fake = Fake::default();
		let older = week("2026-10-30", "2026-11-05");
		fake.ok("index.json", &index(&[("2026-10-30", "2026-11-05")]));
		fake.ok("2026-10-30.json", &older);
		let first = run_once(&pool, &fake, dir.path(), &clock).await.unwrap();
		assert_eq!((first.added, first.exported), (1, 1));
		let file = dir.path().join("2026-10-30.json");
		assert_eq!(std::fs::read(&file).unwrap(), older);

		std::fs::write(&file, b"touched by hand").unwrap();
		fake.answer("index.json", Err(NoAnswer("timeout")));
		let again = run_once(&pool, &fake, dir.path(), &clock).await.unwrap();
		assert_eq!((again.exported, again.failures.as_slice()), (0, &["timeout"][..]));
		assert_eq!(std::fs::read(&file).unwrap(), b"touched by hand", "a written week is left alone");

		std::fs::remove_file(&file).unwrap();
		assert_eq!(run_once(&pool, &fake, dir.path(), &clock).await.unwrap().exported, 1, "a missing file is written again");
		assert_eq!(std::fs::read(&file).unwrap(), older);
		let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|entry| entry.unwrap().file_name()).collect();
		assert_eq!(names, ["2026-10-30.json"], "no partial file is left");

		// The owner records the week's link and lets its video go: its file
		// leaves, and no pass writes it again (CW8).
		let mut conn = pool.acquire().await.unwrap();
		call_week_repo::video::record_link(&mut conn, "2026-10-30", "dQw4w9WgXcQ", "2026-11-07T12:00:00.000Z")
			.await
			.unwrap();
		let receipt = call_week_repo::video::Receipt {
			from: "2026-10-30".to_owned(),
			step: call_week_repo::video::Step::Video,
			youtube_id: "dQw4w9WgXcQ".to_owned(),
			size: 1,
			deleted_at: "2026-11-07T12:00:00.000Z".to_owned(),
		};
		call_week_repo::video::write_receipt(&mut conn, &receipt).await.unwrap();
		drop(conn);
		let after = run_once(&pool, &fake, dir.path(), &clock).await.unwrap();
		assert_eq!(after.exported, 0, "a week let go is not written again");
		assert!(!file.exists(), "and a file left of it is removed");
	}

	#[test]
	fn a_failing_source_is_retried_sooner_but_never_more_often_than_it_is_read() {
		use crate::call_weeks::{INTERVAL, RETRY};
		let now = clock();
		let mut schedule = Target::now(now);
		assert!(schedule.due(now));
		let mut waits = Vec::new();
		for _ in 0..5 {
			schedule.failed(now, RETRY, INTERVAL);
			waits.push((0..=86_400).find(|&s| schedule.due(now + chrono::Duration::seconds(s))).unwrap());
		}
		assert_eq!(waits, [600, 1200, 2400, 3600, 3600]);
		schedule.succeeded(now, INTERVAL);
		assert!(!schedule.due(now + chrono::Duration::seconds(3599)) && schedule.due(now + chrono::Duration::seconds(3600)));
		schedule.failed(now, RETRY, INTERVAL);
		assert!(schedule.due(now + chrono::Duration::seconds(600)), "a good pass resets the count");
	}
}
