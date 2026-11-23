//! A public pages bundle's check, [`super::Family::Pages`]'s
//! (paulgsc/some-ui#3395, some-ui's `docs/snips.md`, "How public pages reach
//! home").
//!
//! `pages-bundle-<run>` holds `pages.json` (`some-ui.pages/1`,
//! `@some-ui/video-render`'s `page.ts`), `SHA256SUMS` over it and every
//! picture, and each picture, `<slug>-<hash12>.png`, and nothing else. It is
//! checked whole, or refused whole: the schema; `SHA256SUMS` naming exactly
//! `pages.json` and every record's `image.name`, and the bundle holding
//! exactly those and `SHA256SUMS`; every file its sum; every record's
//! `bytes` and `sha256` its file's, with its name's hash the sum's start;
//! and no picture named twice. A name is a plain file name
//! ([`PAGE_FILE`]), so nothing is staged outside its folder.

use super::{read, stage_file, sums, zip_error, Why, MAX_FILE, MAX_FILES, MAX_UNPACKED};
use regex::Regex;
use serde::Deserialize;
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::LazyLock;

/// What `pages.json` must be; home refuses any other.
pub const SCHEMA: &str = "some-ui.pages/1";

/// The bundle's record of its pages.
pub const RECORD: &str = "pages.json";

/// A page's picture's name (`@some-ui/video-render`'s `PAGE_FILE`, the
/// same pattern): a plain file name, never a path.
pub static PAGE_FILE: LazyLock<Regex> = LazyLock::new(|| {
	#[allow(clippy::unwrap_used)]
	Regex::new(r"^[a-z0-9][a-z0-9-]{0,95}-([0-9a-f]{12})\.png$").unwrap()
});

#[derive(Deserialize)]
struct Bundle<'a> {
	schema: String,
	#[serde(borrow)]
	pages: Vec<&'a RawValue>,
}

#[derive(Deserialize)]
struct Record {
	image: Image,
}

#[derive(Deserialize)]
struct Image {
	name: String,
	bytes: u64,
	sha256: String,
}

/// Whether `name` is a page's picture whose name's hash starts `sha256`.
fn named_for(name: &str, sha256: &str) -> bool {
	let plain = sha256.len() == 64 && sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
	plain
		&& PAGE_FILE
			.captures(name)
			.and_then(|parts| parts.get(1))
			.is_some_and(|hash| sha256.starts_with(hash.as_str()))
}

/// Checks the bundle at `zip`, as the module doc says, and stages its
/// `pages.json` and pictures in `stage` under their names.
pub(super) fn open(zip: &Path, stage: &Path) -> Result<(), Why> {
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
	let bundle: Bundle<'_> = serde_json::from_slice(&record_bytes).map_err(|_| Why::Refused)?;
	if bundle.schema != SCHEMA {
		return Err(Why::Refused);
	}
	let mut named = BTreeSet::from([String::from(RECORD)]);
	let mut images = Vec::with_capacity(bundle.pages.len());
	for raw in &bundle.pages {
		let record: Record = serde_json::from_str(raw.get()).map_err(|_| Why::Refused)?;
		let image = record.image;
		let ok =
			named_for(&image.name, &image.sha256) && sums.get(&image.name) == Some(&image.sha256) && (1..=MAX_FILE).contains(&image.bytes) && named.insert(image.name.clone());
		if !ok {
			return Err(Why::Refused);
		}
		images.push(image);
	}
	if named.iter().ne(sums.keys()) {
		return Err(Why::Refused);
	}
	if images.iter().map(|image| image.bytes).sum::<u64>() > MAX_UNPACKED {
		return Err(Why::Refused);
	}
	let listed: Result<Vec<String>, _> = archive.file_names().map(|entry| entry.map(std::borrow::Cow::into_owned)).collect();
	let held: BTreeSet<String> = listed.map_err(|_| Why::Refused)?.into_iter().filter(|name| !name.ends_with('/')).collect();
	if held.len() != named.len() + 1 || !held.contains("SHA256SUMS") || !named.iter().all(|name| held.contains(name)) {
		return Err(Why::Refused);
	}
	for image in &images {
		let bytes = read(&mut archive, &image.name)?;
		if bytes.len() as u64 != image.bytes || hex::encode(Sha256::digest(&bytes)) != image.sha256 {
			return Err(Why::Refused);
		}
		stage_file(stage, &image.name, &bytes)?;
	}
	stage_file(stage, RECORD, &record_bytes)
}
