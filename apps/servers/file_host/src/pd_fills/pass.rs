//! One pass of Playoff Delta's fills: read `paulgsc/testing`'s
//! `playoff-delta/index.json`, then fetch and keep each fill it lists that
//! home does not hold, with its season file.
//!
//! - **Only what is missing.** A pass is one request for the index, plus two
//!   per listed fill home holds no fill of that episode for (the fill and its
//!   season, the season once per pass). A fill home holds is never fetched
//!   again.
//! - **Checked before kept.** The index must read as
//!   `some-ui.pd-narration-index/1`, each fill a season of four digits, a
//!   team of two or three capitals, a week from 0 to 99 and a SHA-256. A
//!   fill's file must hash to that SHA-256 and read as
//!   `some-ui.pd-narration/1` for exactly that episode, with a format
//!   version; its season file must read as that season and team's. Each is
//!   at most [`MAX_BODY`] bytes. The fill's full shape and words are the
//!   show's (`@some-ui/playoff-delta`'s `checkNarration`), held by the
//!   routine before the fill merged and by the recorder before it is said;
//!   home keeps the bytes as served.
//! - **Counts and kinds, never bytes.** What went wrong is a kind
//!   ([`Pass::failures`]): no URL, body or fill's words reaches a log.
//! - **No network in tests.** Fetching is `game_feed`'s [`Fetch`] port.

use crate::game_feed::pass::{fetch_capped, Fetch};
use chrono::{DateTime, SecondsFormat, Utc};
use pd_fill_repo::{Fill, Kept};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::collections::{BTreeMap, HashSet};

/// Where the fills are: `paulgsc/testing`'s `playoff-delta/` folder on
/// `main`, public, served without a token.
pub const SOURCE: &str = "https://raw.githubusercontent.com/paulgsc/testing/main/playoff-delta/";

/// What `playoff-delta/index.json` is.
pub const INDEX_SCHEMA: &str = "some-ui.pd-narration-index/1";

/// What a fill's file is.
pub const FILL_SCHEMA: &str = "some-ui.pd-narration/1";

/// The most bytes home takes for the index, a fill or a season: a fill is
/// about 5 KB and a season about 60 KB today.
pub const MAX_BODY: usize = 1_048_576;

/// What a pass did: the fills it added, and each thing that went wrong, by
/// kind.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Pass {
	/// Fills stored this pass.
	pub added: usize,
	/// Listed fills home already held a fill of that episode for.
	pub held: usize,
	/// Each failure, by kind: `timeout`, `connect`, `transport`, `refused`
	/// (a status other than 200), `too_big`, and `unreadable` for an index,
	/// fill or season that does not check out.
	pub failures: Vec<&'static str>,
}

/// A fill the index lists.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Listed {
	pub season: String,
	pub team: String,
	pub week: i64,
	/// Lowercase hex SHA-256 of the fill's file.
	pub sha256: String,
}

impl Listed {
	/// The fill's path under [`SOURCE`]: `<season>/narration/<team>-week-NN.json`.
	fn fill_path(&self) -> String {
		let mut path = self.season.clone() + "/narration/" + &self.team + "-week-";
		if self.week < 10 {
			path.push('0');
		}
		path + &self.week.to_string() + ".json"
	}

	/// Its season's path under [`SOURCE`]: `<season>/<team>.json`.
	fn season_path(&self) -> String {
		self.season.clone() + "/" + &self.team + ".json"
	}
}

fn is_season(text: &str) -> bool {
	text.len() == 4 && text.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_team(text: &str) -> bool {
	(2..=3).contains(&text.len()) && text.bytes().all(|byte| byte.is_ascii_uppercase())
}

fn is_sha256(text: &str) -> bool {
	text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// One listed fill of an index, when it reads as one. Each part is held to
/// digits or capitals, so a path made of them names one file.
fn listed(entry: &Value) -> Option<Listed> {
	let (season, team) = (entry.get("season")?.as_str()?, entry.get("team")?.as_str()?);
	let (week, sha256) = (entry.get("week")?.as_i64()?, entry.get("sha256")?.as_str()?);
	(is_season(season) && is_team(team) && (0..=99).contains(&week) && is_sha256(sha256)).then(|| Listed {
		season: season.to_owned(),
		team: team.to_owned(),
		week,
		sha256: sha256.to_owned(),
	})
}

/// The fills `playoff-delta/index.json` lists, or `None` when it does not
/// read as `some-ui.pd-narration-index/1`.
#[must_use]
pub fn read_index(body: &[u8]) -> Option<Vec<Listed>> {
	let index: Value = serde_json::from_slice(body).ok()?;
	if index.get("schema")?.as_str()? != INDEX_SCHEMA {
		return None;
	}
	index.get("fills")?.as_array()?.iter().map(listed).collect()
}

/// The format version of a fill's file, when it hashes to what the index
/// says and reads as `some-ui.pd-narration/1` for exactly the listed episode.
#[must_use]
pub fn fill_version(body: &[u8], listed: &Listed) -> Option<i64> {
	if hex::encode(Sha256::digest(body)) != listed.sha256 {
		return None;
	}
	let fill: Value = serde_json::from_slice(body).ok()?;
	let ours = fill.get("schema")?.as_str()? == FILL_SCHEMA
		&& fill.get("season")?.as_str()? == listed.season
		&& fill.get("team")?.as_str()? == listed.team
		&& fill.get("week")?.as_i64()? == listed.week;
	let version = fill.get("format")?.get("version")?.as_i64()?;
	(ours && version >= 1).then_some(version)
}

/// Whether a season file reads as the listed season and team's.
#[must_use]
pub fn is_season_of(body: &[u8], listed: &Listed) -> bool {
	let Ok(season) = serde_json::from_slice::<Value>(body) else {
		return false;
	};
	season.get("label").and_then(Value::as_str) == Some(listed.season.as_str()) && season.get("team").and_then(Value::as_str) == Some(listed.team.as_str())
}

/// Fetches `path` under [`SOURCE`] ([`fetch_capped`] at [`MAX_BODY`]),
/// pushing a failure's kind onto `pass`.
async fn fetched<F: Fetch>(fetch: &F, path: &str, pass: &mut Pass) -> Option<Vec<u8>> {
	fetch_capped(fetch, &(String::from(SOURCE) + path), MAX_BODY)
		.await
		.map_err(|kind| pass.failures.push(kind))
		.ok()
}

/// One pass of the fills.
///
/// It reads the index, then fetches and keeps each fill it lists that home
/// holds no fill of that episode for, oldest first. A fill that fails is
/// left for the next pass; the others are still taken.
///
/// # Errors
/// A storage failure, which ends the pass.
pub async fn run_once<F: Fetch>(db: &SqlitePool, fetch: &F, clock: &(dyn Fn() -> DateTime<Utc> + Sync)) -> Result<Pass, sqlx::Error> {
	let mut pass = Pass::default();
	let Some(body) = fetched(fetch, "index.json", &mut pass).await else {
		return Ok(pass);
	};
	let Some(mut fills) = read_index(&body) else {
		pass.failures.push("unreadable");
		return Ok(pass);
	};
	let held: HashSet<(String, String, i64)> = {
		let mut conn = db.acquire().await?;
		pd_fill_repo::held(&mut conn).await?.into_iter().map(|fill| (fill.season, fill.team, fill.week)).collect()
	};
	fills.sort();
	fills.dedup_by(|a, b| (&a.season, &a.team, a.week) == (&b.season, &b.team, b.week));
	// Each season file this pass fetched, by its path: `None` if it failed.
	let mut seasons: BTreeMap<String, Option<Vec<u8>>> = BTreeMap::new();
	for listed in fills {
		if held.contains(&(listed.season.clone(), listed.team.clone(), listed.week)) {
			pass.held += 1;
			continue;
		}
		let Some(body) = fetched(fetch, &listed.fill_path(), &mut pass).await else {
			continue;
		};
		let Some(version) = fill_version(&body, &listed) else {
			pass.failures.push("unreadable");
			continue;
		};
		let path = listed.season_path();
		if !seasons.contains_key(&path) {
			let season = match fetched(fetch, &path, &mut pass).await {
				Some(season) if is_season_of(&season, &listed) => Some(season),
				Some(_) => {
					pass.failures.push("unreadable");
					None
				}
				None => None,
			};
			seasons.insert(path.clone(), season);
		}
		let Some(Some(season_body)) = seasons.get(&path) else {
			continue;
		};
		let taken_at = clock().to_rfc3339_opts(SecondsFormat::Millis, true);
		let fill = Fill {
			season: &listed.season,
			team: &listed.team,
			week: listed.week,
			version,
			body: &body,
			season_body,
			taken_at: &taken_at,
		};
		let mut conn = db.acquire().await?;
		match pd_fill_repo::keep(&mut conn, &fill).await? {
			Kept::Added => pass.added += 1,
			Kept::Held => pass.held += 1,
		}
	}
	Ok(pass)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::game_feed::pass::{Answer, NoAnswer, Target};
	use sqlx::sqlite::SqlitePoolOptions;
	use std::collections::HashMap;
	use std::sync::Mutex;

	/// Answers each path under [`SOURCE`] from a table, and records every
	/// request.
	#[derive(Default)]
	struct Fake {
		answers: Mutex<HashMap<String, Result<Answer, NoAnswer>>>,
		asked: Mutex<Vec<String>>,
	}

	impl Fake {
		fn answer(&self, path: &str, answer: Result<Answer, NoAnswer>) {
			self.answers.lock().unwrap().insert(String::from(SOURCE) + path, answer);
		}

		fn ok(&self, path: &str, body: &[u8]) {
			self.answer(path, Ok(Answer { status: 200, body: body.to_vec() }));
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
		DateTime::parse_from_rfc3339("2026-11-10T20:00:00.000Z").unwrap().with_timezone(&Utc)
	}

	fn fill(week: i64, version: i64) -> Vec<u8> {
		serde_json::json!({
			"schema": FILL_SCHEMA, "season": "2026", "team": "SF", "week": week,
			"format": { "id": "playoff-delta-week", "version": version }, "scenes": []
		})
		.to_string()
		.into_bytes()
	}

	fn season() -> Vec<u8> {
		br#"{"version":6,"label":"2026","team":"SF","weeks":[]}"#.to_vec()
	}

	fn sum(body: &[u8]) -> String {
		hex::encode(Sha256::digest(body))
	}

	fn index(fills: &[(i64, &[u8])]) -> Vec<u8> {
		let fills: Vec<Value> = fills
			.iter()
			.map(|(week, body)| serde_json::json!({ "season": "2026", "team": "SF", "week": week, "sha256": sum(body) }))
			.collect();
		serde_json::json!({ "schema": INDEX_SCHEMA, "fills": fills }).to_string().into_bytes()
	}

	#[tokio::test]
	async fn each_listed_fill_home_lacks_is_fetched_once_and_kept_with_its_season() {
		let (pool, fake) = (pool().await, Fake::default());
		let (legend, eight) = (fill(0, 1), fill(8, 2));
		fake.ok("index.json", &index(&[(8, &eight), (0, &legend)]));
		fake.ok("2026/narration/SF-week-00.json", &legend);
		fake.ok("2026/narration/SF-week-08.json", &eight);
		fake.ok("2026/SF.json", &season());

		let first = run_once(&pool, &fake, &clock).await.unwrap();
		assert_eq!((first.added, first.held, first.failures.len()), (2, 0, 0));
		assert_eq!(
			fake.asked(),
			["index.json", "2026/narration/SF-week-00.json", "2026/SF.json", "2026/narration/SF-week-08.json"],
			"oldest first, the season once"
		);
		let mut conn = pool.acquire().await.unwrap();
		let held = pd_fill_repo::held(&mut conn).await.unwrap();
		assert_eq!(held.iter().map(|fill| (fill.week, fill.version)).collect::<Vec<_>>(), [(0, 1), (8, 2)]);
		assert_eq!(pd_fill_repo::bodies(&mut conn, "2026", "SF", 8).await.unwrap(), Some((eight, season())));
		drop(conn);

		let again = run_once(&pool, &fake, &clock).await.unwrap();
		assert_eq!((again.added, again.held), (0, 2));
		assert_eq!(fake.asked(), ["index.json"], "a fill home holds is never fetched again");
	}

	#[tokio::test]
	async fn what_does_not_check_out_is_counted_by_kind_and_left_for_the_next_pass() {
		let (pool, fake) = (pool().await, Fake::default());
		assert_eq!(run_once(&pool, &fake, &clock).await.unwrap().failures, ["connect"]);
		fake.answer("index.json", Ok(Answer { status: 404, body: vec![] }));
		assert_eq!(run_once(&pool, &fake, &clock).await.unwrap().failures, ["refused"]);
		fake.ok("index.json", b"<html>busy</html>");
		assert_eq!(run_once(&pool, &fake, &clock).await.unwrap().failures, ["unreadable"]);
		let good = serde_json::json!({ "season": "2026", "team": "SF", "week": 1, "sha256": sum(b"x") });
		for bad in [
			serde_json::json!({ "season": "../x", "team": "SF", "week": 1, "sha256": sum(b"x") }),
			serde_json::json!({ "season": "2026", "team": "sf", "week": 1, "sha256": sum(b"x") }),
			serde_json::json!({ "season": "2026", "team": "SF/", "week": 1, "sha256": sum(b"x") }),
			serde_json::json!({ "season": "2026", "team": "SF", "week": 100, "sha256": sum(b"x") }),
			serde_json::json!({ "season": "2026", "team": "SF", "week": 1.5, "sha256": sum(b"x") }),
			serde_json::json!({ "season": "2026", "team": "SF", "week": 1, "sha256": "ABC" }),
		] {
			let body = serde_json::json!({ "schema": INDEX_SCHEMA, "fills": [good, bad] }).to_string();
			fake.ok("index.json", body.as_bytes());
			assert_eq!(run_once(&pool, &fake, &clock).await.unwrap().failures, ["unreadable"], "{bad}");
		}
		fake.asked();

		let (one, two, three, four) = (fill(1, 2), fill(2, 2), fill(3, 2), fill(4, 2));
		fake.ok("index.json", &index(&[(1, &one), (2, &two), (3, &three), (4, &four)]));
		fake.ok("2026/narration/SF-week-01.json", &vec![b' '; MAX_BODY + 1]);
		fake.ok("2026/narration/SF-week-02.json", &fill(2, 3));
		fake.ok("2026/narration/SF-week-03.json", &fill(4, 2));
		fake.answer("2026/narration/SF-week-04.json", Err(NoAnswer("timeout")));
		let pass = run_once(&pool, &fake, &clock).await.unwrap();
		assert_eq!(
			pass.failures,
			["too_big", "unreadable", "unreadable", "timeout"],
			"a changed fill or another week's is not kept"
		);
		assert_eq!(pass.added, 0);

		fake.ok("2026/narration/SF-week-04.json", &four);
		fake.ok("2026/SF.json", br#"{"label":"2025","team":"SF"}"#);
		assert_eq!(run_once(&pool, &fake, &clock).await.unwrap().failures[3], "unreadable", "another season's file");
		fake.ok("2026/SF.json", &season());
		fake.asked();
		let next = run_once(&pool, &fake, &clock).await.unwrap();
		assert_eq!(next.added, 1, "the good fill is taken");
		assert_eq!(
			fake.asked(),
			[
				"index.json",
				"2026/narration/SF-week-01.json",
				"2026/narration/SF-week-02.json",
				"2026/narration/SF-week-03.json",
				"2026/narration/SF-week-04.json",
				"2026/SF.json"
			],
			"each failed fill is asked again"
		);
	}

	#[test]
	fn a_failing_source_is_retried_sooner_but_never_more_often_than_it_is_read() {
		use crate::pd_fills::{INTERVAL, RETRY};
		let now = clock();
		let mut schedule = Target::now(now);
		let mut waits = Vec::new();
		for _ in 0..5 {
			schedule.failed(now, RETRY, INTERVAL);
			waits.push((0..=86_400).find(|&s| schedule.due(now + chrono::Duration::seconds(s))).unwrap());
		}
		assert_eq!(waits, [600, 1200, 2400, 3600, 3600]);
	}
}
