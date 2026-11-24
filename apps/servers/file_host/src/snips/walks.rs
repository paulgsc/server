//! A walks bundle's check, [`super::Family::Walks`]'s (paulgsc/some-ui#3476).
//!
//! `walks-bundle-<run>`, uploaded by some-ui's `Snips Capture` for one day,
//! holds `walks.json` (`some-ui.walks/1`), `SHA256SUMS` over it and every
//! picture, and each walk's pictures, `<app>/snips/<NN>.png` (two digits
//! from 01, the walk's steps in order), and nothing else. A walk that failed
//! in that run is left out, never listed half.
//!
//! It is checked whole, or refused whole: the schema, the run's own commit
//! and id; each app a slug ([`APP`]) listed once, its walk a
//! `some-ui.walk/1` of that app with at least one step and at most
//! [`MOST_STEPS`], each step's `file` `snips/<NN>.png` in order, and its
//! `pictures` exactly those, `<app>/snips/<NN>.png`, in the same order;
//! `SHA256SUMS` naming exactly `walks.json` and every picture, and the bundle
//! holding exactly those and `SHA256SUMS`; every file its sum, and every
//! picture's `bytes` and `sha256` its file's. A name is only ever an app's
//! slug and a step's number, so nothing is staged outside its folder.

use super::{read, stage_file, sums, zip_error, Run, Why, MAX_FILE, MAX_FILES, MAX_UNPACKED};
use regex::Regex;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::LazyLock;

/// What `walks.json` must be; home refuses any other.
pub const SCHEMA: &str = "some-ui.walks/1";

/// What each walk in it must be (`@some-ui/video-motion`'s `WalkFileSchema`).
pub const WALK_SCHEMA: &str = "some-ui.walk/1";

/// The bundle's record of its walks.
pub const RECORD: &str = "walks.json";

/// The most steps a walk has: its pictures' numbers are two digits.
pub const MOST_STEPS: usize = 99;

/// An app's slug, as a scene names it (`@some-ui/video-render`'s `showId`,
/// the same pattern).
pub static APP: LazyLock<Regex> = LazyLock::new(|| {
	#[allow(clippy::unwrap_used)]
	Regex::new(r"^[a-z0-9][a-z0-9-]{0,63}$").unwrap()
});

/// A step's file in its walk, `snips/<NN>.png`, step `n` counted from 1.
#[must_use]
pub fn step_file(n: usize) -> String {
	String::from("snips/") + if n < 10 { "0" } else { "" } + &n.to_string() + ".png"
}

/// A step's picture in the bundle, `<app>/snips/<NN>.png`.
#[must_use]
pub fn picture_of(app: &str, n: usize) -> String {
	String::from(app) + "/" + &step_file(n)
}

#[derive(Deserialize)]
struct Bundle {
	schema: String,
	commit: String,
	run: String,
	walks: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
	app: String,
	walk: Walk,
	pictures: Vec<Picture>,
}

/// What of a walk the check reads; the rest is the render's to read.
#[derive(Deserialize)]
struct Walk {
	schema: String,
	app: String,
	snips: Vec<Step>,
}

#[derive(Deserialize)]
struct Step {
	file: String,
}

#[derive(Deserialize)]
struct Picture {
	name: String,
	bytes: u64,
	sha256: String,
}

/// Whether `entry` is one walk of the bundle as the module doc says,
/// against `sums`.
fn walk_checks_out(entry: &Entry, sums: &std::collections::BTreeMap<String, String>) -> bool {
	let steps = entry.walk.snips.len();
	APP.is_match(&entry.app)
		&& entry.walk.schema == WALK_SCHEMA
		&& entry.walk.app == entry.app
		&& (1..=MOST_STEPS).contains(&steps)
		&& entry.pictures.len() == steps
		&& entry.walk.snips.iter().zip(&entry.pictures).enumerate().all(|(at, (step, picture))| {
			step.file == step_file(at + 1)
				&& picture.name == picture_of(&entry.app, at + 1)
				&& sums.get(&picture.name) == Some(&picture.sha256)
				&& (1..=MAX_FILE).contains(&picture.bytes)
		})
}

/// Checks the bundle at `zip` against `run`, as the module doc says, and
/// stages its `walks.json` and pictures in `stage` under their names.
pub(super) fn open(zip: &Path, stage: &Path, run: &Run) -> Result<(), Why> {
	let file = std::fs::File::open(zip).map_err(|_| Why::Drive)?;
	let mut archive = zip::ZipArchive::new(file).map_err(zip_error)?;
	if archive.len() > MAX_FILES {
		return Err(Why::Refused);
	}
	let sums = sums(std::str::from_utf8(&read(&mut archive, "SHA256SUMS")?).map_err(|_| Why::Refused)?).ok_or(Why::Refused)?;
	let record_bytes = read(&mut archive, RECORD)?;
	if sums.get(RECORD) != Some(&hex::encode(Sha256::digest(&record_bytes))) {
		return Err(Why::Refused);
	}
	let bundle: Bundle = serde_json::from_slice(&record_bytes).map_err(|_| Why::Refused)?;
	if bundle.schema != SCHEMA || bundle.commit != run.head_sha || !super::is_commit(&bundle.commit) || bundle.run != run.id.to_string() {
		return Err(Why::Refused);
	}
	let mut named = BTreeSet::from([String::from(RECORD)]);
	let mut apps = BTreeSet::new();
	for entry in &bundle.walks {
		if !walk_checks_out(entry, &sums) || !apps.insert(entry.app.as_str()) {
			return Err(Why::Refused);
		}
		named.extend(entry.pictures.iter().map(|picture| picture.name.clone()));
	}
	if named.iter().ne(sums.keys()) {
		return Err(Why::Refused);
	}
	let pictures: Vec<&Picture> = bundle.walks.iter().flat_map(|entry| &entry.pictures).collect();
	if pictures.iter().map(|picture| picture.bytes).sum::<u64>() > MAX_UNPACKED {
		return Err(Why::Refused);
	}
	let listed: Result<Vec<String>, _> = archive.file_names().map(|entry| entry.map(std::borrow::Cow::into_owned)).collect();
	let held: BTreeSet<String> = listed.map_err(|_| Why::Refused)?.into_iter().filter(|name| !name.ends_with('/')).collect();
	if held.len() != named.len() + 1 || !held.contains("SHA256SUMS") || !named.iter().all(|name| held.contains(name)) {
		return Err(Why::Refused);
	}
	for picture in pictures {
		let bytes = read(&mut archive, &picture.name)?;
		if bytes.len() as u64 != picture.bytes || hex::encode(Sha256::digest(&bytes)) != picture.sha256 {
			return Err(Why::Refused);
		}
		stage_file(stage, &picture.name, &bytes)?;
	}
	stage_file(stage, RECORD, &record_bytes)
}
