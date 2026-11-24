//! Walks, one kind of [`super`]: our own apps in motion, a walk's pictures
//! from a run of some-ui's `Snips Capture` (paulgsc/some-ui#3476).
//!
//! A cut list's picture `{"kind": "screen", "of": {"from": "walks", "run":
//! "<digits>", "app": "<slug>", "steps": [first, last]?}}` names one by the
//! run that walked it and its app, and the steps it shows, counted from 1 as
//! the pictures are (`<NN>`); no `steps` is the whole walk. Its item is
//! `<run> <app>`, then ` <first>-<last>` when it names steps (` ?` when they
//! don't read as such); it is named by `<run> <app>` alone ([`id_of`]), in a
//! refusal as in a wait.
//!
//! Home hands them beside the essay in `<key>.walks/` (C1472): one `walks.json`
//! (`some-ui.walks-given/1`, [`given`]) holding, for each run the essay
//! names, in the order first named, its commit and the records of only the
//! apps it names, as the bundle has them (each once, its steps all of them,
//! the render taking the steps from the scene); and each of those walks'
//! pictures at `<run>/<app>/snips/<NN>.png`. The render's `--walks` checks
//! every picture's sha256 again.
//!
//! The pictures come from the bundle `crate::snips` fetches for a waiting
//! essay into its cache ([`Sources::walks`]): while a run's bundle is not
//! there yet the essay waits; a run marked gone refuses it (`walk-gone`, C1471), as
//! does an app the run's `walks.json` doesn't list or a last step past its
//! walk's: a walk left out failed in that run, and waiting won't bring it.
//! A scene naming no run, app or steps a capture makes is missing
//! (`walk-missing`), as is every walk when home fetches none (no
//! `SNIPS_GITHUB_TOKEN`); a refusal is looked at again every pass. A walk
//! is no one's record, so it never takes an essay back: its copy goes with
//! the video.

use super::{Failed, File, Held, Sources, Why};
use crate::snips::walks::{picture_of, APP, RECORD};
use serde_json::Value;
use sqlx::SqliteConnection;

/// What `walks.json` given beside an essay is.
pub const GIVEN_SCHEMA: &str = "some-ui.walks-given/1";

/// The walk a cut list's `shot` shows, as `<run> <app>[ <first>-<last>]`.
#[must_use]
pub fn named(shot: &Value) -> Vec<String> {
	let Some(of) = super::screen_of(shot, "walks") else {
		return Vec::new();
	};
	let part = |name| of.get(name).and_then(Value::as_str).unwrap_or_default();
	let mut item = String::from(part("run")) + " " + part("app");
	match of.get("steps") {
		None => {}
		Some(steps) => {
			let pair = steps
				.as_array()
				.filter(|pair| pair.len() == 2)
				.and_then(|pair| Some((pair[0].as_u64()?, pair[1].as_u64()?)));
			item += &pair.map_or_else(|| String::from(" ?"), |(first, last)| String::from(" ") + &first.to_string() + "-" + &last.to_string());
		}
	}
	vec![item]
}

/// How a walk's `item` is named: `<run> <app>`.
#[must_use]
pub fn id_of(item: &str) -> String {
	item.splitn(3, ' ').take(2).collect::<Vec<_>>().join(" ")
}

/// Which of a walk's steps a scene shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Steps {
	/// No `steps`: the whole walk.
	All,
	/// `first` to `last`, counted from 1.
	Some(usize, usize),
}

impl Steps {
	/// The steps an item names, or `None` when they are no steps a walk
	/// has: from 1, first no later than last.
	fn of(steps: Option<&str>) -> Option<Self> {
		let Some(steps) = steps else {
			return Some(Self::All);
		};
		let (first, last) = steps.split_once('-')?;
		let (first, last) = (first.parse::<usize>().ok()?, last.parse::<usize>().ok()?);
		(1 <= first && first <= last).then_some(Self::Some(first, last))
	}

	/// Whether a walk of `count` steps has them.
	const fn within(self, count: usize) -> bool {
		match self {
			Self::All => true,
			Self::Some(_, last) => last <= count,
		}
	}
}

/// One walk, `<run> <app>[ <first>-<last>]`, as the module doc says. Its
/// pictures are read only when `read`: a walk already held for another
/// scene's steps is checked again, never read again, so an essay holds
/// each walk's pictures once however many scenes show it.
pub(super) async fn hold(_conn: &mut SqliteConnection, sources: &Sources, item: &str, read: bool) -> Result<Held, Failed> {
	let mut parts = item.splitn(3, ' ');
	let (run, app) = (parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
	let Some(steps) = Steps::of(parts.next()) else {
		return Ok(Held::Refused(Why::Missing));
	};
	if !crate::snips::is_run(run) || !APP.is_match(app) {
		return Ok(Held::Refused(Why::Missing));
	}
	let Some(folder) = sources.walks.as_deref() else {
		return Ok(Held::Refused(Why::Missing));
	};
	let bundle = crate::snips::bundle_of(folder, run);
	if !bundle.is_dir() {
		return Ok(if crate::snips::gone_of(folder, run).exists() {
			Held::Refused(Why::Gone)
		} else {
			Held::Waiting
		});
	}
	// The bundle was checked whole when fetched: one with no record is not
	// what home cached, and is missing, as a snip's is.
	let Some(record) = super::read_held(&bundle.join(RECORD)).await? else {
		return Ok(Held::Refused(Why::Missing));
	};
	let record: Value = serde_json::from_slice(&record).map_err(|_| Failed::Unreadable)?;
	let commit = record.get("commit").and_then(Value::as_str).unwrap_or_default();
	let walks = record.get("walks").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
	let Some(entry) = walks.iter().find(|entry| entry.get("app").and_then(Value::as_str) == Some(app)) else {
		return Ok(Held::Refused(Why::Gone));
	};
	let count = entry.get("walk").and_then(|walk| walk.get("snips")).and_then(Value::as_array).map_or(0, Vec::len);
	if !steps.within(count) {
		return Ok(Held::Refused(Why::Gone));
	}
	let listed = entry.get("pictures").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
	if listed.len() != count {
		return Ok(Held::Refused(Why::Missing));
	}
	let mut pictures = Vec::with_capacity(if read { count } else { 0 });
	for (at, picture) in listed.iter().enumerate().filter(|_| read) {
		// Its name is the one the check held it to, never read from the
		// record: it reads only inside the bundle.
		let name = picture_of(app, at + 1);
		let Some(bytes) = super::read_held(&bundle.join(&name)).await? else {
			return Ok(Held::Refused(Why::Missing));
		};
		if picture.get("name").and_then(Value::as_str) != Some(name.as_str()) || Some(super::sha256_hex(&bytes).as_str()) != picture.get("sha256").and_then(Value::as_str) {
			return Ok(Held::Refused(Why::Missing));
		}
		pictures.push((String::from(run) + "/" + &name, bytes));
	}
	Ok(Held::Record {
		bundle: String::from(run),
		record: serde_json::json!({ "commit": commit, "walk": entry }),
		pictures,
	})
}

/// Whether home still holds a walk: always. A walk is no one's record, and
/// home never takes an essay back for one.
#[allow(clippy::unused_async)]
pub(super) async fn still_held(_conn: &mut SqliteConnection, _sources: &Sources, _item: &str) -> Result<bool, sqlx::Error> {
	Ok(true)
}

/// `walks.json`: for each run, in the order first named, its commit and the
/// records of the apps the essay names, each once; then each of their
/// pictures. None for none. Each of `walked` is a run, `{commit, walk}` as
/// [`hold`] gives it, and the walk's pictures.
pub(super) fn given(walked: Vec<(String, Value, Vec<File>)>) -> Vec<File> {
	if walked.is_empty() {
		return Vec::new();
	}
	let mut runs: Vec<(String, Value, Vec<Value>)> = Vec::new();
	let mut pictures: Vec<File> = Vec::new();
	for (run, record, files) in walked {
		let commit = record.get("commit").cloned().unwrap_or(Value::Null);
		let Some(walk) = record.get("walk").cloned() else {
			continue;
		};
		if !runs.iter().any(|(held, _, _)| *held == run) {
			runs.push((run.clone(), commit, Vec::new()));
		}
		let Some((_, _, walks)) = runs.iter_mut().find(|(held, _, _)| *held == run) else {
			continue;
		};
		if walks.iter().any(|held| held.get("app") == walk.get("app")) {
			continue;
		}
		walks.push(walk);
		pictures.extend(files);
	}
	let runs: Vec<Value> = runs
		.into_iter()
		.map(|(run, commit, walks)| serde_json::json!({ "run": run, "commit": commit, "walks": walks }))
		.collect();
	let body = serde_json::json!({ "schema": GIVEN_SCHEMA, "runs": runs });
	std::iter::once((String::from(RECORD), body.to_string().into_bytes())).chain(pictures).collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use sqlx::Connection;

	#[tokio::test]
	async fn a_walk_held_again_is_checked_not_read_again() {
		let dir = tempfile::tempdir().unwrap();
		let bundle = crate::snips::bundle_of(dir.path(), "301");
		std::fs::create_dir_all(bundle.join("jobs/snips")).unwrap();
		let walks = serde_json::json!({ "schema": "some-ui.walks/1", "commit": "c".repeat(40), "run": "301", "walks": [{
			"app": "jobs",
			"walk": { "schema": "some-ui.walk/1", "app": "jobs", "snips": [{ "file": "snips/01.png" }, { "file": "snips/02.png" }] },
			"pictures": [
				{ "name": "jobs/snips/01.png", "bytes": 1, "sha256": super::super::sha256_hex(b"1") },
				{ "name": "jobs/snips/02.png", "bytes": 1, "sha256": super::super::sha256_hex(b"2") },
			],
		}]});
		std::fs::write(bundle.join(RECORD), walks.to_string()).unwrap();
		std::fs::write(bundle.join("jobs/snips/01.png"), b"1").unwrap();
		std::fs::write(bundle.join("jobs/snips/02.png"), b"2").unwrap();
		let sources = Sources {
			subject: None,
			frames: dir.path().join("no-frames"),
			snips: None,
			pages: None,
			walks: Some(dir.path().to_owned()),
		};
		let mut conn = SqliteConnection::connect("sqlite::memory:").await.unwrap();
		let Held::Record { pictures, .. } = hold(&mut conn, &sources, "301 jobs 1-1", true).await.unwrap() else {
			panic!("held");
		};
		assert_eq!(pictures.len(), 2, "every picture, once");
		// With its pictures gone, a walk held again still checks out: none is read.
		std::fs::remove_dir_all(bundle.join("jobs")).unwrap();
		let Held::Record { pictures, .. } = hold(&mut conn, &sources, "301 jobs 2-2", false).await.unwrap() else {
			panic!("checked");
		};
		assert!(pictures.is_empty());
		assert_eq!(
			hold(&mut conn, &sources, "301 jobs 2-3", false).await.unwrap(),
			Held::Refused(Why::Gone),
			"its steps are still checked"
		);
	}
}
