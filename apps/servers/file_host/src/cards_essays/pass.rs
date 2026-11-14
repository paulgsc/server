//! One pass of the cards essays.
//!
//! It reads `paulgsc/testing`'s `cards/index.json`, fetches and keeps each
//! essay it lists that home does not hold, then writes each essay home holds
//! that the render worker's folder lacks.
//!
//! - **Only what is missing.** A pass is one request for the index, plus one
//!   per listed essay home holds no essay of that key of. An essay home holds
//!   is never fetched again.
//! - **No index yet is no essays.** Until the first essay merges,
//!   `paulgsc/testing` has no `cards/index.json`: a `404` for it is a pass
//!   with nothing to take, not a failure, so a fresh home does not alert.
//! - **Checked before kept.** The index must read as
//!   `some-ui.cards-index/1`, each essay `{"key": …}` with a key
//!   [`is_key`] takes, `<slug>-<YYYY-MM-DD>`. An entry that isn't one (a day
//!   that is no calendar day, say, which `cards-check`'s pattern lets
//!   through) is skipped and counted, and the others are still taken. An essay's file must be a
//!   cut list (`some-ui.cut-list/…`) whose stamp's day is its key's, and be
//!   at most [`MAX_BODY`] bytes. Its full shape is the renderer's own
//!   (`@some-ui/video-render`'s `parseCardsEssay`), held by the routine's
//!   `cards-check` before the essay merged and again by `record:cards`
//!   before anything is drawn; home keeps the bytes as served.
//! - **Counts and kinds, never bytes.** What went wrong is a kind
//!   ([`Pass::failures`]): no URL, body or essay's words reaches a log.
//! - **No network in tests.** Fetching is `game_feed`'s [`Fetch`] port.

use crate::game_feed::pass::{fetch_capped, Answer, Fetch, NoAnswer};
use cards_essay_repo::{Essay, Kept};
use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use serde_json::Value;
use sqlx::SqlitePool;
use std::collections::HashSet;
use std::path::Path;

/// Where the essays are: `paulgsc/testing`'s `cards/` folder on `main`,
/// public, served without a token.
pub const SOURCE: &str = "https://raw.githubusercontent.com/paulgsc/testing/main/cards/";

/// What `cards/index.json` is (`@some-ui/video-render`'s
/// `CARDS_INDEX_SCHEMA`).
pub const INDEX_SCHEMA: &str = "some-ui.cards-index/1";

/// What an essay's file's schema starts with: a cut list, of any version.
pub const CUT_LIST_SCHEMA: &str = "some-ui.cut-list/";

/// The most bytes home takes for the index or an essay: an essay is a few KB
/// (the two showcases are under 20 KB), and the mailbox's own cap is the same.
pub const MAX_BODY: usize = 1_048_576;

/// The longest key home takes, as `cards_essay`'s check does.
pub const MAX_KEY: usize = 128;

/// What a pass did: the essays it added, and each thing that went wrong, by
/// kind.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Pass {
	/// Essays stored this pass.
	pub added: usize,
	/// Listed essays home already held.
	pub held: usize,
	/// Essays written to the render worker's folder this pass.
	pub exported: usize,
	/// Each failure, by kind: `timeout`, `connect`, `transport`, `refused`
	/// (a status other than 200), `too_big`, `unreadable`, and `export`
	/// for an essay that could not be written to the folder.
	pub failures: Vec<&'static str>,
}

/// Whether `text` is a calendar day written `YYYY-MM-DD`, and nothing else:
/// chrono's parser also takes a sign, a space or a one-digit field, so the
/// day must print back as `text`.
fn is_day(text: &str) -> bool {
	NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok_and(|day| day.format("%Y-%m-%d").to_string() == text)
}

/// The day of an essay's key, when it is one.
///
/// A key is `<slug>-<YYYY-MM-DD>`: a slug of plain lowercase words joined by
/// `-` (`@some-ui/video-render`'s `CARDS_KEY`), then a calendar day, at most
/// [`MAX_KEY`] long. So a key is always one plain file name.
#[must_use]
pub fn day_of(key: &str) -> Option<&str> {
	if key.len() > MAX_KEY || !key.is_ascii() {
		return None;
	}
	let (slug, day) = key.split_at(key.len().checked_sub(10)?);
	let slug = slug.strip_suffix('-')?;
	let plain = |word: &str| !word.is_empty() && word.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit());
	(slug.split('-').all(plain) && is_day(day)).then_some(day)
}

/// Whether `key` is an essay's key ([`day_of`]).
#[must_use]
pub fn is_key(key: &str) -> bool {
	day_of(key).is_some()
}

/// What `cards/index.json` lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Index {
	/// Each entry that is `{"key": <an essay's key>}`.
	pub keys: Vec<String>,
	/// Entries that are not, each skipped.
	pub bad: usize,
}

/// What `cards/index.json` lists, or `None` when it does not read as
/// `some-ui.cards-index/1` (a schema, and an `essays` list).
#[must_use]
pub fn read_index(body: &[u8]) -> Option<Index> {
	let index: Value = serde_json::from_slice(body).ok()?;
	if index.get("schema")?.as_str()? != INDEX_SCHEMA {
		return None;
	}
	let mut read = Index { keys: Vec::new(), bad: 0 };
	for entry in index.get("essays")?.as_array()? {
		let key = entry
			.as_object()
			.filter(|entry| entry.len() == 1)
			.and_then(|entry| entry.get("key"))
			.and_then(Value::as_str)
			.filter(|key| is_key(key));
		match key {
			Some(key) => read.keys.push(key.to_owned()),
			None => read.bad += 1,
		}
	}
	Some(read)
}

/// Whether an essay's file reads as a cut list stamped on its key's day.
#[must_use]
pub fn is_essay(body: &[u8], key: &str) -> bool {
	let Ok(essay) = serde_json::from_slice::<Value>(body) else {
		return false;
	};
	let cut_list = essay.get("schema").and_then(Value::as_str).is_some_and(|schema| schema.starts_with(CUT_LIST_SCHEMA));
	let stamped = essay.get("stamp").and_then(|stamp| stamp.get("date")).and_then(Value::as_str);
	cut_list && stamped.is_some() && stamped == day_of(key)
}

/// Fetches `url` ([`fetch_capped`] at [`MAX_BODY`]), pushing a failure's
/// kind onto `pass`.
async fn fetched<F: Fetch>(fetch: &F, url: &str, pass: &mut Pass) -> Option<Vec<u8>> {
	fetch_capped(fetch, url, MAX_BODY).await.map_err(|kind| pass.failures.push(kind)).ok()
}

/// One pass: [`take_new`], then [`export`].
///
/// It takes each essay the repository lists that home lacks, then writes
/// each essay home holds that `folder` lacks, so the render worker sees
/// every essay, a restart included.
///
/// # Errors
/// A storage failure, which ends the pass.
pub async fn run_once<F: Fetch>(db: &SqlitePool, fetch: &F, folder: &Path, clock: &(dyn Fn() -> DateTime<Utc> + Sync)) -> Result<Pass, sqlx::Error> {
	let mut pass = Pass::default();
	take_new(db, fetch, clock, &mut pass).await?;
	export(db, folder, &mut pass).await?;
	Ok(pass)
}

/// Reads the index, then fetches and keeps each essay it lists that home
/// holds none of, oldest day first. An essay that fails is left for the next
/// pass; the others are still taken.
async fn take_new<F: Fetch>(db: &SqlitePool, fetch: &F, clock: &(dyn Fn() -> DateTime<Utc> + Sync), pass: &mut Pass) -> Result<(), sqlx::Error> {
	let body = match fetch.get_at_most(&(String::from(SOURCE) + "index.json"), MAX_BODY).await {
		Ok(Answer { status: 200, body }) => body,
		// No essay has merged yet: nothing to take.
		Ok(Answer { status: 404, .. }) => return Ok(()),
		Ok(_) => {
			pass.failures.push("refused");
			return Ok(());
		}
		Err(NoAnswer(kind)) => {
			pass.failures.push(kind);
			return Ok(());
		}
	};
	let Some(Index { mut keys, bad }) = read_index(&body) else {
		pass.failures.push("unreadable");
		return Ok(());
	};
	pass.failures.extend(std::iter::repeat_n("unreadable", bad));
	let held: HashSet<String> = {
		let mut conn = db.acquire().await?;
		cards_essay_repo::held(&mut conn).await?.into_iter().map(|essay| essay.key).collect()
	};
	// By day, not by name: a slug-first key's name order isn't its day's.
	keys.sort_by(|a, b| (day_of(a), a).cmp(&(day_of(b), b)));
	keys.dedup();
	for key in keys {
		if held.contains(&key) {
			pass.held += 1;
			continue;
		}
		let Some(day) = day_of(&key) else { continue };
		// `key` is checked plain, so the URL names one file.
		let Some(body) = fetched(fetch, &(String::from(SOURCE) + &key + ".json"), pass).await else {
			continue;
		};
		if !is_essay(&body, &key) {
			pass.failures.push("unreadable");
			continue;
		}
		let taken_at = clock().to_rfc3339_opts(SecondsFormat::Millis, true);
		let essay = Essay {
			key: &key,
			day,
			body: &body,
			taken_at: &taken_at,
		};
		let mut conn = db.acquire().await?;
		match cards_essay_repo::keep(&mut conn, &essay).await? {
			Kept::Added => pass.added += 1,
			Kept::Held => pass.held += 1,
		}
	}
	Ok(())
}

/// Writes each essay home holds that `folder` has no file of, as
/// `<key>.json`, byte for byte, through a temporary name so the render
/// worker never reads half a file. An essay already written is left alone:
/// home never rewrites a kept essay, so neither does this.
async fn export(db: &SqlitePool, folder: &Path, pass: &mut Pass) -> Result<(), sqlx::Error> {
	let held = {
		let mut conn = db.acquire().await?;
		cards_essay_repo::held(&mut conn).await?
	};
	let missing: Vec<_> = held.into_iter().filter(|essay| !folder.join(String::from(&essay.key) + ".json").exists()).collect();
	if missing.is_empty() {
		return Ok(());
	}
	if tokio::fs::create_dir_all(folder).await.is_err() {
		pass.failures.push("export");
		return Ok(());
	}
	for essay in missing {
		let body = {
			let mut conn = db.acquire().await?;
			cards_essay_repo::body(&mut conn, &essay.key).await?
		};
		let Some(body) = body else { continue };
		let file = folder.join(String::from(&essay.key) + ".json");
		let partial = folder.join(String::from(".") + &essay.key + ".json.partial");
		let written = tokio::fs::write(&partial, &body).await.is_ok() && tokio::fs::rename(&partial, &file).await.is_ok();
		if written {
			pass.exported += 1;
		} else {
			pass.failures.push("export");
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
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
		DateTime::parse_from_rfc3339("2026-11-14T09:00:00.000Z").unwrap().with_timezone(&Utc)
	}

	fn index(keys: &[&str]) -> Vec<u8> {
		let essays: Vec<Value> = keys.iter().map(|key| serde_json::json!({ "key": key })).collect();
		serde_json::json!({ "schema": INDEX_SCHEMA, "essays": essays }).to_string().into_bytes()
	}

	fn essay(day: &str) -> Vec<u8> {
		serde_json::json!({
			"schema": "some-ui.cut-list/2",
			"title": "A machine upgrades itself",
			"series": { "id": "cheat-sheet", "version": 1 },
			"stamp": { "sources": ["paulgsc/nixos@abc1234"], "date": day },
			"shots": [],
		})
		.to_string()
		.into_bytes()
	}

	#[test]
	fn a_key_is_a_plain_slug_and_a_calendar_day() {
		for key in ["nixos-upgrade-2026-11-14", "a-2026-11-14", "rust2-scratch-pad-2026-02-28"] {
			assert!(is_key(key), "{key}");
		}
		assert_eq!(day_of("nixos-upgrade-2026-11-14"), Some("2026-11-14"));
		for key in [
			"2026-11-14",
			"-2026-11-14",
			"nixos--upgrade-2026-11-14",
			"NixOS-2026-11-14",
			"nix_os-2026-11-14",
			"../etc-2026-11-14",
			"nixos-2026-02-30",
			"nixos-2026-1-14",
			"nixos-+2026-11-14",
			"nixös-2026-11-14",
			"",
		] {
			assert!(!is_key(key), "{key:?} is not a key");
		}
		assert!(!is_key(&(String::from("a").repeat(MAX_KEY) + "-2026-11-14")), "too long");
	}

	#[tokio::test]
	async fn each_listed_essay_home_lacks_is_fetched_once_by_day_and_kept_as_served() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let fake = Fake::default();
		fake.ok("index.json", &index(&["rust-scratchpad-2026-11-14", "nixos-upgrade-2026-11-14", "zfs-2026-11-02"]));
		for (key, day) in [
			("rust-scratchpad-2026-11-14", "2026-11-14"),
			("nixos-upgrade-2026-11-14", "2026-11-14"),
			("zfs-2026-11-02", "2026-11-02"),
		] {
			fake.ok(&(String::from(key) + ".json"), &essay(day));
		}

		let first = run_once(&pool, &fake, dir.path(), &clock).await.unwrap();
		assert_eq!((first.added, first.held, first.exported, first.failures.len()), (3, 0, 3, 0));
		assert_eq!(
			fake.asked(),
			["index.json", "zfs-2026-11-02.json", "nixos-upgrade-2026-11-14.json", "rust-scratchpad-2026-11-14.json"],
			"oldest day first, by key within a day"
		);
		assert_eq!(std::fs::read(dir.path().join("zfs-2026-11-02.json")).unwrap(), essay("2026-11-02"), "byte for byte");

		let again = run_once(&pool, &fake, dir.path(), &clock).await.unwrap();
		assert_eq!((again.added, again.held, again.exported), (0, 3, 0));
		assert_eq!(fake.asked(), ["index.json"], "an essay home holds is never fetched again");
	}

	#[tokio::test]
	async fn what_does_not_check_out_is_counted_by_kind_and_left_for_the_next_pass() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let fake = Fake::default();
		assert_eq!(run_once(&pool, &fake, dir.path(), &clock).await.unwrap().failures, ["connect"]);
		fake.answer(
			"index.json",
			Ok(Answer {
				status: 404,
				body: b"404: Not Found".to_vec(),
			}),
		);
		let none_yet = run_once(&pool, &fake, dir.path(), &clock).await.unwrap();
		assert_eq!(none_yet, Pass::default(), "no index yet is no essays, not a failure");
		fake.answer(
			"index.json",
			Ok(Answer {
				status: 503,
				body: b"busy".to_vec(),
			}),
		);
		assert_eq!(run_once(&pool, &fake, dir.path(), &clock).await.unwrap().failures, ["refused"]);
		fake.ok("index.json", b"<html>busy</html>");
		assert_eq!(run_once(&pool, &fake, dir.path(), &clock).await.unwrap().failures, ["unreadable"]);
		fake.ok("index.json", br#"{"schema":"some-ui.call-index/1","essays":[]}"#);
		assert_eq!(run_once(&pool, &fake, dir.path(), &clock).await.unwrap().failures, ["unreadable"], "another index");
		fake.ok("index.json", br#"{"schema":"some-ui.cards-index/1"}"#);
		assert_eq!(run_once(&pool, &fake, dir.path(), &clock).await.unwrap().failures, ["unreadable"], "no essays list");
		// An entry that isn't an essay's key is skipped and counted; the
		// others are still taken.
		fake.ok("zfs-2026-11-02.json", &essay("2026-11-02"));
		for key in ["../secrets-2026-11-14", "NixOS-2026-11-14", "nixos-2026-02-30"] {
			fake.ok("index.json", &index(&["zfs-2026-11-02", key]));
			let pass = run_once(&pool, &fake, dir.path(), &clock).await.unwrap();
			assert_eq!(pass.failures, ["unreadable"], "{key:?} is not a key");
			assert!(
				fake
					.asked()
					.iter()
					.all(|asked| !asked.contains("secrets") && !asked.contains("NixOS") && !asked.contains("02-30")),
				"{key:?} is never fetched"
			);
		}
		fake.ok("index.json", br#"{"schema":"some-ui.cards-index/1","essays":[{"key":"zfs-2026-11-02","title":"x"}]}"#);
		assert_eq!(
			run_once(&pool, &fake, dir.path(), &clock).await.unwrap().failures,
			["unreadable"],
			"an entry is its key alone"
		);
		assert_eq!(
			cards_essay_repo::held(&mut pool.acquire().await.unwrap()).await.unwrap().len(),
			1,
			"the good essay was taken beside the bad entries"
		);
		fake.asked();

		fake.ok("index.json", &index(&["d-2026-11-14", "c-2026-11-13", "b-2026-11-12", "a-2026-11-11", "e-2026-11-15"]));
		fake.ok("a-2026-11-11.json", &vec![b' '; MAX_BODY + 1]);
		fake.ok("b-2026-11-12.json", &essay("2026-11-11"));
		fake.answer("c-2026-11-13.json", Err(NoAnswer("timeout")));
		fake.ok("d-2026-11-14.json", &essay("2026-11-14"));
		fake.ok("e-2026-11-15.json", br#"{"schema":"some-ui.call-week/1","stamp":{"date":"2026-11-15"}}"#);
		let pass = run_once(&pool, &fake, dir.path(), &clock).await.unwrap();
		assert_eq!(
			pass.failures,
			["too_big", "unreadable", "timeout", "unreadable"],
			"too big, stamped another day, no answer, not a cut list"
		);
		assert_eq!((pass.added, pass.exported), (1, 1), "the good essay is still taken");

		fake.ok("c-2026-11-13.json", &essay("2026-11-13"));
		fake.asked();
		let next = run_once(&pool, &fake, dir.path(), &clock).await.unwrap();
		assert_eq!(next.added, 1);
		assert_eq!(
			fake.asked(),
			["index.json", "a-2026-11-11.json", "b-2026-11-12.json", "c-2026-11-13.json", "e-2026-11-15.json"],
			"each failed essay is asked again"
		);
	}

	/// The render worker's folder gets each essay home holds, byte for byte,
	/// once: an essay already there is never rewritten, and a pass whose index
	/// can't be read still writes what home holds.
	#[tokio::test]
	async fn every_essay_held_is_written_for_the_render_worker_once() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let fake = Fake::default();
		fake.ok("index.json", &index(&["zfs-2026-11-02"]));
		fake.ok("zfs-2026-11-02.json", &essay("2026-11-02"));
		assert_eq!(run_once(&pool, &fake, dir.path(), &clock).await.unwrap().exported, 1);
		let file = dir.path().join("zfs-2026-11-02.json");

		std::fs::write(&file, b"touched by hand").unwrap();
		fake.answer("index.json", Err(NoAnswer("timeout")));
		let again = run_once(&pool, &fake, dir.path(), &clock).await.unwrap();
		assert_eq!((again.exported, again.failures.as_slice()), (0, &["timeout"][..]));
		assert_eq!(std::fs::read(&file).unwrap(), b"touched by hand", "a written essay is left alone");

		std::fs::remove_file(&file).unwrap();
		assert_eq!(run_once(&pool, &fake, dir.path(), &clock).await.unwrap().exported, 1, "a missing file is written again");
		assert_eq!(std::fs::read(&file).unwrap(), essay("2026-11-02"));
		let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|entry| entry.unwrap().file_name()).collect();
		assert_eq!(names, ["zfs-2026-11-02.json"], "no partial file is left");
	}

	/// What the render worker made reads as rendered, waiting or failed, as
	/// every job's.
	#[tokio::test]
	async fn what_the_worker_made_of_each_essay_is_counted() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let fake = Fake::default();
		fake.ok("index.json", &index(&["a-2026-11-11", "b-2026-11-12", "c-2026-11-13"]));
		for (key, day) in [("a-2026-11-11", "2026-11-11"), ("b-2026-11-12", "2026-11-12"), ("c-2026-11-13", "2026-11-13")] {
			fake.ok(&(String::from(key) + ".json"), &essay(day));
		}
		run_once(&pool, &fake, &dir.path().join("essays"), &clock).await.unwrap();
		let (renders, status) = (dir.path().join("renders"), dir.path().join("status"));
		std::fs::create_dir_all(renders.join("a-2026-11-11")).unwrap();
		std::fs::write(renders.join("a-2026-11-11").join(crate::cards_essays::VIDEO), b"mp4").unwrap();
		std::fs::create_dir_all(&status).unwrap();
		std::fs::write(status.join("b-2026-11-12.failed.log"), b"refused").unwrap();
		std::fs::write(status.join("last-pass"), b"1763110800\n").unwrap();

		let seen = crate::cards_essays::observe(&pool, &renders, &status).await.unwrap();
		assert_eq!((seen.held, seen.waiting, seen.failed, seen.let_go), (3, 2, 1, 0));
		assert_eq!(seen.worker_last_pass, Some(1_763_110_800));
		assert_eq!(seen.oldest_waiting_taken, Some(clock().timestamp()));
	}
}
