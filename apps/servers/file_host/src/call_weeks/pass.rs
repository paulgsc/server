//! One pass of the call's weeks: read `paulgsc/testing`'s `call/index.json`,
//! then fetch and keep each week it lists that home does not hold.
//!
//! It also takes a newer revision of each week it holds while no part of
//! that week has a video.
//!
//! - **Only what can change.** A pass is one request for the index, plus one
//!   per listed week whose first day home holds no week of, plus one per
//!   listed week home holds that is still open: no part of it has a video
//!   (no link recorded, no `call.mp4` on the drive) and the render worker is
//!   not attempting it. A week with a video is never fetched again: it is
//!   frozen at the revision its video was made from. So the idle pass, every
//!   listed week with its video, is still the one request an hour; a week
//!   waiting for its video costs one more an hour until it has it, at most
//!   the index's listed weeks (its `keep`).
//! - **Appended, never rewritten** (paulgsc/server#879). An open week whose
//!   served bytes differ from the newest revision home keeps is stored as
//!   its next revision (`call_week_repo::revise`); the first stays in
//!   `call_week` as it was. The worker's week file becomes the newest
//!   revision, written only while no part of the week has a video, looked
//!   at again just before it is written.
//! - **The worker owns the count of parts** (C380). This never touches a
//!   week's `.parts`: the worker keeps beside it the checksum of the file it
//!   counted (`.parts-of`) and asks again only while no part has a video, so
//!   one actor alone decides, and a series is never read as fewer parts once
//!   one of its parts was made (paulgsc/server#874). A render under way when
//!   the file changed is never kept: the worker compares the file's checksum
//!   before and after (`render-calls.sh`'s `revision`).
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
use call_week_repo::{Kept, Revised, Week};
use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use serde_json::Value;
use sqlx::SqlitePool;
use std::collections::HashSet;
use std::path::Path;

/// The folders a pass reads and writes: the render worker's input, where it
/// keeps its videos and `.parts`, and where it says what it is attempting.
#[derive(Debug, Clone, Copy)]
pub struct Folders<'a> {
	/// Where each week home holds is written ([`super::RENDER_FOLDER`]).
	pub weeks: &'a Path,
	/// Where the worker keeps each part's video ([`super::render::RENDERS_FOLDER`]).
	pub renders: &'a Path,
	/// Where the worker says how its attempts go ([`super::render::STATUS_FOLDER`]).
	pub status: &'a Path,
}

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
	/// Newer revisions of weeks home holds stored this pass.
	pub revised: usize,
	/// Listed weeks home already held, at the revision served or frozen.
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

/// One pass: [`open_weeks`], [`take_new`], then [`export`].
///
/// It takes each week the repository lists that home lacks and each newer
/// revision of an open week, then writes each week home holds that the
/// worker's folder lacks or holds an older revision of, so the render worker
/// sees every week at its newest revision, a restart included.
///
/// # Errors
/// A storage failure, which ends the pass.
pub async fn run_once<F: Fetch>(db: &SqlitePool, fetch: &F, folders: Folders<'_>, clock: &(dyn Fn() -> DateTime<Utc> + Sync)) -> Result<Pass, sqlx::Error> {
	let mut pass = Pass::default();
	let weeks = open_weeks(db, folders).await?;
	take_new(db, fetch, clock, &weeks.open, &mut pass).await?;
	export(db, folders, &weeks.unmade, &mut pass).await?;
	Ok(pass)
}

/// The weeks home holds that no video was made of, and of those the ones
/// that may take a revision this pass.
#[derive(Debug, Default)]
struct Weeks {
	/// No part has a link recorded (a receipt needs one), and no part's
	/// `call.mp4` is in `renders`: the worker's file of such a week is kept
	/// at its newest revision, once [`no_video_now`] says so again.
	unmade: HashSet<String>,
	/// Of those, the ones the worker is attempting none of the parts of
	/// (`.<key>.log` in `status`): only these are asked for again, so a
	/// render under way is not thrown away for a revision.
	open: HashSet<String>,
}

/// [`Weeks`], read from the record and the worker's folders.
///
/// A folder that cannot be read closes every week, as a video that cannot
/// be seen must not be taken for none; one that is not there yet has no
/// video or attempt in it.
async fn open_weeks(db: &SqlitePool, folders: Folders<'_>) -> Result<Weeks, sqlx::Error> {
	let (held, links) = {
		let mut conn = db.acquire().await?;
		(call_week_repo::held(&mut conn).await?, call_week_repo::video::links(&mut conn).await?)
	};
	let mut open: HashSet<String> = held.into_iter().map(|week| week.from).collect();
	for key in links.keys() {
		if let Some((week, _)) = call_week_repo::video::split_key(key) {
			open.remove(week);
		}
	}
	if open.is_empty() {
		return Ok(Weeks::default());
	}
	let Some(made) = names_in(folders.renders).await else {
		return Ok(Weeks::default());
	};
	for name in made {
		if let Some((week, _)) = call_week_repo::video::split_key(&name) {
			// A video that can't be looked at is taken for one.
			if open.contains(week) && folders.renders.join(&name).join(super::render::VIDEO).try_exists().unwrap_or(true) {
				open.remove(week);
			}
		}
	}
	let unmade = open.clone();
	let Some(status) = names_in(folders.status).await else {
		return Ok(Weeks { unmade, open: HashSet::new() });
	};
	for name in status {
		let attempt = name.strip_prefix('.').and_then(|rest| rest.strip_suffix(".log"));
		if let Some((week, _)) = attempt.and_then(call_week_repo::video::split_key) {
			open.remove(week);
		}
	}
	Ok(Weeks { unmade, open })
}

/// Whether no part of the week of `from` has a video, looked at now: no link
/// recorded, and no part's `call.mp4` in `renders`. A folder or a file that
/// can't be read says there may be one.
async fn no_video_now(db: &SqlitePool, renders: &Path, from: &str) -> Result<bool, sqlx::Error> {
	let links = {
		let mut conn = db.acquire().await?;
		call_week_repo::video::links(&mut conn).await?
	};
	if links.keys().any(|key| call_week_repo::video::split_key(key).is_some_and(|(week, _)| week == from)) {
		return Ok(false);
	}
	let Some(made) = names_in(renders).await else {
		return Ok(false);
	};
	Ok(
		!made
			.iter()
			.any(|name| call_week_repo::video::split_key(name).is_some_and(|(week, _)| week == from) && renders.join(name).join(super::render::VIDEO).try_exists().unwrap_or(true)),
	)
}

/// The names in `folder`; none when it is not there, `None` when it cannot
/// be read.
async fn names_in(folder: &Path) -> Option<Vec<String>> {
	let mut entries = match tokio::fs::read_dir(folder).await {
		Ok(entries) => entries,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Some(Vec::new()),
		Err(_) => return None,
	};
	let mut names = Vec::new();
	loop {
		match entries.next_entry().await {
			Ok(Some(entry)) => names.push(entry.file_name().to_string_lossy().into_owned()),
			Ok(None) => return Some(names),
			Err(_) => return None,
		}
	}
}

/// Writes each week home holds that `folder` has no file of, as
/// `<from>.json`, its newest revision byte for byte, through a temporary
/// name so the render worker never reads half a file. A week already written
/// is left alone, unless it took a revision and no video was made of it
/// (`unmade`): its file is then compared with the newest revision, and
/// written again if it differs and no part has a video still, looked at
/// again just before (paulgsc/server#879). The worker asks how many parts
/// the new file is itself (C380); this never touches `.parts`.
///
/// A week whose video the owner let go from My data (`feeds::videos::let_go`) is
/// never written, and its file is removed if one is left, so the worker
/// never renders it again (CW8). A week that is a series of parts
/// (paulgsc/some-ui#2918) is let go so only once every part's video is;
/// until then each part let go is marked beside its file
/// (`call_weeks::LET_GO`), and a mark that is missing is written again.
async fn export(db: &SqlitePool, folders: Folders<'_>, unmade: &HashSet<String>, pass: &mut Pass) -> Result<(), sqlx::Error> {
	let folder = folders.weeks;
	// Held for the whole pass, so a week let go while it runs is never
	// written back (`feeds::FOLDERS`).
	let _folders = crate::feeds::FOLDERS.lock().await;
	let (held, let_go, parts_gone) = {
		let mut conn = db.acquire().await?;
		(
			call_week_repo::held(&mut conn).await?,
			call_week_repo::video::weeks_let_go(&mut conn).await?,
			call_week_repo::video::let_go(&mut conn).await?,
		)
	};
	for from in &let_go {
		match tokio::fs::remove_file(folder.join(String::from(from) + ".json")).await {
			Err(error) if error.kind() != std::io::ErrorKind::NotFound => pass.failures.push("export"),
			_ => {}
		}
	}
	for key in &parts_gone {
		let Some((week, _)) = call_week_repo::video::split_key(key) else { continue };
		let mark = folder.join(String::from(key) + super::LET_GO);
		let done = if let_go.iter().any(|gone| gone == week) {
			// The week went whole: its marks go with its file.
			match tokio::fs::remove_file(&mark).await {
				Err(error) => error.kind() == std::io::ErrorKind::NotFound,
				Ok(()) => true,
			}
		} else {
			crate::feeds::videos::mark_let_go(folder, key).await.is_ok()
		};
		if !done {
			pass.failures.push("export");
		}
	}
	// Each week to write, and whether it is a newer revision of one written.
	let mut due = Vec::new();
	for week in held.into_iter().filter(|week| !let_go.contains(&week.from)) {
		let file = folder.join(String::from(&week.from) + ".json");
		let revised = week.revision > 1 && unmade.contains(&week.from);
		if !file.exists() {
			due.push((week, revised));
		} else if revised {
			let body = {
				let mut conn = db.acquire().await?;
				call_week_repo::body(&mut conn, &week.from).await?
			};
			if tokio::fs::read(&file).await.ok() != body {
				due.push((week, true));
			}
		}
	}
	if due.is_empty() {
		return Ok(());
	}
	if tokio::fs::create_dir_all(folder).await.is_err() {
		pass.failures.push("export");
		return Ok(());
	}
	for (week, revised) in due {
		let body = {
			let mut conn = db.acquire().await?;
			call_week_repo::body(&mut conn, &week.from).await?
		};
		let Some(body) = body else { continue };
		let file = folder.join(String::from(&week.from) + ".json");
		let partial = folder.join(String::from(".") + &week.from + ".json.partial");
		// A part made since the pass began freezes the week at the file its
		// video was made from.
		if revised && !no_video_now(db, folders.renders, &week.from).await? {
			continue;
		}
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
/// holds no week of, and each newer revision of a listed week that is
/// `open`, oldest first. A week that fails is left for the next pass; the
/// others are still taken.
async fn take_new<F: Fetch>(db: &SqlitePool, fetch: &F, clock: &(dyn Fn() -> DateTime<Utc> + Sync), open: &HashSet<String>, pass: &mut Pass) -> Result<(), sqlx::Error> {
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
		let is_held = held.contains(&listed.from);
		if is_held && !open.contains(&listed.from) {
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
		if is_held {
			match call_week_repo::revise(&mut conn, &week).await {
				Ok(Revised::Taken(_)) => pass.revised += 1,
				Ok(Revised::Same | Revised::NotHeld) => pass.held += 1,
				// A link recorded since the pass looked: the week froze, as
				// the table says.
				Err(error) if call_week_repo::is_frozen(&error) => pass.held += 1,
				Err(error) => return Err(error),
			}
		} else {
			match call_week_repo::keep(&mut conn, &week).await? {
				Kept::Added => pass.added += 1,
				Kept::Held => pass.held += 1,
			}
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

	/// The worker's three folders, apart under one temporary root.
	struct Dirs {
		_root: tempfile::TempDir,
		weeks: std::path::PathBuf,
		renders: std::path::PathBuf,
		status: std::path::PathBuf,
	}

	impl Dirs {
		fn new() -> Self {
			let root = tempfile::tempdir().unwrap();
			let (weeks, renders, status) = (root.path().join("weeks"), root.path().join("call"), root.path().join("call-worker"));
			std::fs::create_dir_all(&weeks).unwrap();
			Self {
				_root: root,
				weeks,
				renders,
				status,
			}
		}

		fn path(&self) -> &Path {
			&self.weeks
		}

		fn folders(&self) -> Folders<'_> {
			Folders {
				weeks: &self.weeks,
				renders: &self.renders,
				status: &self.status,
			}
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
		let (pool, dir) = (pool().await, Dirs::new());
		let fake = Fake::default();
		fake.ok("index.json", &index(&[("2026-11-06", "2026-11-12"), ("2026-10-30", "2026-11-05")]));
		let (older, newer) = (week("2026-10-30", "2026-11-05"), week("2026-11-06", "2026-11-12"));
		fake.ok("2026-10-30.json", &older);
		fake.ok("2026-11-06.json", &newer);

		let first = run_once(&pool, &fake, dir.folders(), &clock).await.unwrap();
		assert_eq!((first.added, first.held, first.failures.len()), (2, 0, 0));
		assert_eq!(fake.asked(), ["index.json", "2026-10-30.json", "2026-11-06.json"], "oldest first");

		let again = run_once(&pool, &fake, dir.folders(), &clock).await.unwrap();
		assert_eq!((again.added, again.held), (0, 2));
		assert_eq!(
			fake.asked(),
			["index.json", "2026-10-30.json", "2026-11-06.json"],
			"a week with no video yet is asked again, for a re-stamp"
		);
		assert_eq!(std::fs::read(dir.path().join("2026-10-30.json")).unwrap(), older, "kept as served");
	}

	#[tokio::test]
	async fn what_does_not_check_out_is_counted_by_kind_and_left_for_the_next_pass() {
		let (pool, dir) = (pool().await, Dirs::new());
		let fake = Fake::default();
		assert_eq!(run_once(&pool, &fake, dir.folders(), &clock).await.unwrap().failures, ["connect"]);
		fake.answer(
			"index.json",
			Ok(Answer {
				status: 429,
				body: b"slow down".to_vec(),
			}),
		);
		assert_eq!(run_once(&pool, &fake, dir.folders(), &clock).await.unwrap().failures, ["refused"]);
		fake.ok("index.json", b"<html>busy</html>");
		assert_eq!(run_once(&pool, &fake, dir.folders(), &clock).await.unwrap().failures, ["unreadable"]);
		for day in ["../secrets", "+2026-1-05", " 2026-1-05", "2026-02-30"] {
			fake.ok("index.json", &index(&[("2026-10-30", "2026-11-05"), (day, "2026-11-05")]));
			assert_eq!(
				run_once(&pool, &fake, dir.folders(), &clock).await.unwrap().failures,
				["unreadable"],
				"{day:?} is not a day"
			);
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
		let pass = run_once(&pool, &fake, dir.folders(), &clock).await.unwrap();
		assert_eq!(pass.failures, ["too_big", "unreadable", "timeout"]);
		assert_eq!(pass.added, 1, "the good week is still taken");

		fake.ok("2026-11-06.json", &week("2026-11-06", "2026-11-12"));
		fake.asked();
		let next = run_once(&pool, &fake, dir.folders(), &clock).await.unwrap();
		assert_eq!(next.added, 1);
		assert_eq!(
			fake.asked(),
			["index.json", "2026-10-23.json", "2026-10-30.json", "2026-11-06.json", "2026-11-13.json"],
			"each failed week is asked again, and the one held with no video yet"
		);
	}

	/// The render worker's folder gets each week home holds, byte for byte,
	/// once: a week already there is never rewritten, and a pass whose index
	/// can't be read still writes what home holds.
	#[tokio::test]
	async fn every_week_held_is_written_for_the_render_worker_once() {
		let (pool, dir) = (pool().await, Dirs::new());
		let fake = Fake::default();
		let older = week("2026-10-30", "2026-11-05");
		fake.ok("index.json", &index(&[("2026-10-30", "2026-11-05")]));
		fake.ok("2026-10-30.json", &older);
		let first = run_once(&pool, &fake, dir.folders(), &clock).await.unwrap();
		assert_eq!((first.added, first.exported), (1, 1));
		let file = dir.path().join("2026-10-30.json");
		assert_eq!(std::fs::read(&file).unwrap(), older);

		std::fs::write(&file, b"touched by hand").unwrap();
		fake.answer("index.json", Err(NoAnswer("timeout")));
		let again = run_once(&pool, &fake, dir.folders(), &clock).await.unwrap();
		assert_eq!((again.exported, again.failures.as_slice()), (0, &["timeout"][..]));
		assert_eq!(std::fs::read(&file).unwrap(), b"touched by hand", "a written week is left alone");

		std::fs::remove_file(&file).unwrap();
		assert_eq!(run_once(&pool, &fake, dir.folders(), &clock).await.unwrap().exported, 1, "a missing file is written again");
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
		call_week_repo::video::write_receipt(&mut conn, &receipt, 1).await.unwrap();
		drop(conn);
		let after = run_once(&pool, &fake, dir.folders(), &clock).await.unwrap();
		assert_eq!(after.exported, 0, "a week let go is not written again");
		assert!(!file.exists(), "and a file left of it is removed");
	}

	/// A week that is a series (paulgsc/some-ui#2918): a part let go is marked
	/// for the worker, a mark lost is written again, and the week's file and
	/// marks leave once every part's video has.
	#[tokio::test]
	async fn a_part_let_go_is_marked_and_the_week_leaves_once_every_part_has() {
		let (pool, dir) = (pool().await, Dirs::new());
		let fake = Fake::default();
		fake.ok("index.json", &index(&[("2026-11-06", "2026-11-12")]));
		fake.ok("2026-11-06.json", &week("2026-11-06", "2026-11-12"));
		run_once(&pool, &fake, dir.folders(), &clock).await.unwrap();
		let (file, mark) = (dir.path().join("2026-11-06.json"), dir.path().join("2026-11-06-p2.let-go"));
		let go = |key: &'static str, id: &'static str| {
			let pool = pool.clone();
			async move {
				let mut conn = pool.acquire().await.unwrap();
				call_week_repo::video::record_link(&mut conn, key, id, "2026-11-14T12:00:00.000Z").await.unwrap();
				let receipt = call_week_repo::video::Receipt {
					from: key.to_owned(),
					step: call_week_repo::video::Step::Video,
					youtube_id: id.to_owned(),
					size: 1,
					deleted_at: "2026-11-14T12:00:00.000Z".to_owned(),
				};
				call_week_repo::video::write_receipt(&mut conn, &receipt, 2).await.unwrap();
			}
		};
		go("2026-11-06-p2", "bbbbbbbbbbb").await;
		run_once(&pool, &fake, dir.folders(), &clock).await.unwrap();
		assert!(file.exists() && mark.exists(), "part 1 still renders; part 2 never again");
		std::fs::remove_file(&mark).unwrap();
		run_once(&pool, &fake, dir.folders(), &clock).await.unwrap();
		assert!(mark.exists(), "a mark lost is written again");

		go("2026-11-06", "aaaaaaaaaaa").await;
		let after = run_once(&pool, &fake, dir.folders(), &clock).await.unwrap();
		assert!(after.failures.is_empty(), "{:?}", after.failures);
		let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|entry| entry.unwrap().file_name()).collect();
		assert!(names.is_empty(), "the week and its marks are gone: {names:?}");
	}

	fn stamped(from: &str, to: &str, template: u32) -> Vec<u8> {
		serde_json::json!({ "schema": WEEK_SCHEMA, "week": { "from": from, "to": to }, "template": template })
			.to_string()
			.into_bytes()
	}

	/// paulgsc/server#879: a week home took at template version 4 and that
	/// the routine re-stamped to 5 is taken again while it has no video.
	/// The worker's file becomes the re-stamp, and the first revision stays
	/// kept. Its `.parts` is the worker's to ask again (C380: the worker
	/// counted another file); this leaves it alone.
	#[tokio::test]
	async fn a_restamp_of_a_week_with_no_video_is_taken_and_rendered_as_parts() {
		let (pool, dir) = (pool().await, Dirs::new());
		let fake = Fake::default();
		let (v4, v5) = (stamped("2026-11-06", "2026-11-12", 4), stamped("2026-11-06", "2026-11-12", 5));
		fake.ok("index.json", &index(&[("2026-11-06", "2026-11-12")]));
		fake.ok("2026-11-06.json", &v4);
		run_once(&pool, &fake, dir.folders(), &clock).await.unwrap();
		// The worker found the v4 week one part and failed it (16.6 min).
		std::fs::create_dir_all(&dir.renders).unwrap();
		let parts = dir.renders.join("2026-11-06.parts");
		std::fs::write(&parts, b"1\n").unwrap();
		std::fs::create_dir_all(&dir.status).unwrap();
		std::fs::write(dir.status.join("2026-11-06.failed.log"), b"refused").unwrap();
		fake.asked();

		let same = run_once(&pool, &fake, dir.folders(), &clock).await.unwrap();
		assert_eq!((same.revised, same.held, same.exported), (0, 1, 0), "the bytes home has are no revision");
		assert_eq!(fake.asked(), ["index.json", "2026-11-06.json"], "a week with no video is asked again");
		assert!(parts.exists(), "nothing changed, so the parts stand");

		fake.ok("2026-11-06.json", &v5);
		let pass = run_once(&pool, &fake, dir.folders(), &clock).await.unwrap();
		assert_eq!((pass.revised, pass.exported, pass.failures.len()), (1, 1, 0), "{:?}", pass.failures);
		assert_eq!(std::fs::read(dir.path().join("2026-11-06.json")).unwrap(), v5, "the worker renders the newest");
		assert_eq!(std::fs::read(&parts).unwrap(), b"1\n", "the count is the worker's");
		let mut conn = pool.acquire().await.unwrap();
		let first: Vec<u8> = sqlx::query_scalar("SELECT body FROM call_week").fetch_one(&mut *conn).await.unwrap();
		assert_eq!(first, v4, "the first revision is kept as it was");
		assert_eq!(call_week_repo::held(&mut conn).await.unwrap()[0].revision, 2);
		drop(conn);

		// A file the worker holds of an older revision is written again,
		// even while an attempt runs: the worker throws that render away.
		let week_file = dir.path().join("2026-11-06.json");
		std::fs::write(&week_file, &v4).unwrap();
		std::fs::write(dir.status.join(".2026-11-06.log"), b"recording").unwrap();
		let next = run_once(&pool, &fake, dir.folders(), &clock).await.unwrap();
		assert_eq!((next.revised, next.exported, next.failures.len()), (0, 1, 0), "{:?}", next.failures);
		assert_eq!(std::fs::read(&week_file).unwrap(), v5);

		// Once a part has its video, the file is frozen at the one it was
		// made from, whatever the record says.
		std::fs::write(&week_file, &v4).unwrap();
		std::fs::create_dir_all(dir.renders.join("2026-11-06")).unwrap();
		std::fs::write(dir.renders.join("2026-11-06").join(super::super::render::VIDEO), b"video").unwrap();
		let made = run_once(&pool, &fake, dir.folders(), &clock).await.unwrap();
		assert_eq!(made.exported, 0);
		assert_eq!(std::fs::read(&week_file).unwrap(), v4, "a week with a video is not written again");
	}

	/// Once any part of a week has a video, made or linked, or is being
	/// made, the week is frozen: it is not asked again, and a re-stamp is
	/// not taken. So the idle pass, every week with its video, stays the one
	/// request for the index.
	#[tokio::test]
	async fn a_week_with_a_video_ignores_a_restamp_and_the_idle_pass_is_one_request() {
		let (pool, dir) = (pool().await, Dirs::new());
		let fake = Fake::default();
		let listed = [("2026-10-30", "2026-11-05"), ("2026-11-06", "2026-11-12"), ("2026-11-13", "2026-11-19")];
		fake.ok("index.json", &index(&listed));
		for (from, to) in listed {
			fake.ok(&(String::from(from) + ".json"), &stamped(from, to, 4));
		}
		run_once(&pool, &fake, dir.folders(), &clock).await.unwrap();
		// 10-30: part 2's video is on the drive. 11-06: a part's link is
		// recorded. 11-13: the worker is attempting its first part.
		std::fs::create_dir_all(dir.renders.join("2026-10-30-p2")).unwrap();
		std::fs::write(dir.renders.join("2026-10-30-p2").join(super::super::render::VIDEO), b"video").unwrap();
		std::fs::write(dir.renders.join("2026-10-30.parts"), b"2\n").unwrap();
		let mut conn = pool.acquire().await.unwrap();
		call_week_repo::video::record_link(&mut conn, "2026-11-06", "aaaaaaaaaaa", "2026-11-14T12:00:00.000Z")
			.await
			.unwrap();
		drop(conn);
		std::fs::create_dir_all(&dir.status).unwrap();
		std::fs::write(dir.status.join(".2026-11-13.log"), b"recording").unwrap();
		for (from, to) in listed {
			fake.ok(&(String::from(from) + ".json"), &stamped(from, to, 5));
		}
		fake.asked();

		let pass = run_once(&pool, &fake, dir.folders(), &clock).await.unwrap();
		assert_eq!((pass.revised, pass.held, pass.exported), (0, 3, 0));
		assert_eq!(fake.asked(), ["index.json"], "the idle pass is one request");
		for (from, to) in listed {
			assert_eq!(std::fs::read(dir.path().join(String::from(from) + ".json")).unwrap(), stamped(from, to, 4));
		}
		assert!(dir.renders.join("2026-10-30.parts").exists(), "a series with a video keeps its parts");

		// The attempt ends with no video: the week is open again.
		std::fs::remove_file(dir.status.join(".2026-11-13.log")).unwrap();
		let next = run_once(&pool, &fake, dir.folders(), &clock).await.unwrap();
		assert_eq!(fake.asked(), ["index.json", "2026-11-13.json"]);
		assert_eq!(next.revised, 1);

		// A drive that can't be read is taken for one with videos.
		std::fs::remove_dir_all(&dir.renders).unwrap();
		std::fs::write(&dir.renders, b"not a folder").unwrap();
		run_once(&pool, &fake, dir.folders(), &clock).await.unwrap();
		assert_eq!(fake.asked(), ["index.json"], "nothing is asked again");
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
