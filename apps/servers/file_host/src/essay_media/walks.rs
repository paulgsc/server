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
			item += &pair.map_or_else(|| String::from(" ?"), |(first, last)| format!(" {first}-{last}"));
		}
	}
	vec![item]
}

/// How a walk's `item` is named: `<run> <app>`.
#[must_use]
pub fn id_of(item: &str) -> String {
	item.splitn(3, ' ').take(2).collect::<Vec<_>>().join(" ")
}

/// The steps an item names, `Some(None)` for the whole walk, or `None` when
/// they are no steps a walk has: from 1, first no later than last.
fn steps_of(steps: Option<&str>) -> Option<Option<(usize, usize)>> {
	let Some(steps) = steps else {
		return Some(None);
	};
	let (first, last) = steps.split_once('-')?;
	let (first, last) = (first.parse::<usize>().ok()?, last.parse::<usize>().ok()?);
	(1 <= first && first <= last).then_some(Some((first, last)))
}

/// One walk, `<run> <app>[ <first>-<last>]`, as the module doc says.
pub(super) async fn hold(_conn: &mut SqliteConnection, sources: &Sources, item: &str) -> Result<Held, Failed> {
	let mut parts = item.splitn(3, ' ');
	let (run, app) = (parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
	let Some(steps) = steps_of(parts.next()) else {
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
	if steps.is_some_and(|(_, last)| last > count) {
		return Ok(Held::Refused(Why::Gone));
	}
	let listed = entry.get("pictures").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
	if listed.len() != count {
		return Ok(Held::Refused(Why::Missing));
	}
	let mut pictures = Vec::with_capacity(count);
	for (at, picture) in listed.iter().enumerate() {
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
		let at = match runs.iter().position(|(held, _, _)| *held == run) {
			Some(at) => at,
			None => {
				runs.push((run, commit, Vec::new()));
				runs.len() - 1
			}
		};
		let walks = &mut runs[at].2;
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
