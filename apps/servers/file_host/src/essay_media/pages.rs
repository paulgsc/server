//! Public pages, one kind of [`super`]: pages anyone can open logged out,
//! photographed by some-ui's `Public Pages Capture` (paulgsc/some-ui#3395).
//!
//! A cut list's picture `{"kind": "screen", "of": {"from": "pages", "url",
//! "capturedAt", "file", "run", "at"?}}` names one by the run that took it
//! and its file there, and says which address it shows and when it was
//! taken. Its item is `<run> <file> <capturedAt> <url>`; it is named by
//! `<run> <file>` alone ([`id_of`]), in a refusal as in a wait.
//!
//! Home hands each beside the essay in `<key>.pages/`: its picture under its
//! own name and one `pages.json` (`some-ui.pages/1`, [`given`]) holding only
//! the records the essay names, gathered from every run it names, one record
//! per file: the render's `--pages`.
//!
//! The pictures come from the bundle `crate::snips` fetches for a waiting
//! essay into its cache ([`Sources::pages`]): while a run's bundle is not
//! there yet the essay waits; a run marked gone refuses it (`page-gone`).
//! A scene's file its run's `pages.json` doesn't hold, or holds of another
//! address or moment (the moments compared as instants, as some-ui's
//! `resolveScreens` reads them), is gone too, naming the run and the file
//! (C1441): a re-attempt of the run may have taken other pictures, or the
//! scene names the wrong run, and a page can't be taken again the same. So is
//! one file two runs give unlike; the same bytes are the same picture, kept
//! once. A scene naming no run or file a capture makes is missing
//! (`page-missing`), as is every page when home fetches none. A page is no
//! one's record, so it never takes an essay back: its copy goes with the
//! video.

use super::{Failed, File, Held, Sources, Why};
use crate::snips::pages::{PAGE_FILE, RECORD, SCHEMA};
use serde_json::Value;
use sqlx::SqliteConnection;

/// The page a cut list's `shot` shows, as `<run> <file> <capturedAt> <url>`.
#[must_use]
pub fn named(shot: &Value) -> Vec<String> {
	let Some(of) = super::screen_of(shot, "pages") else {
		return Vec::new();
	};
	let part = |name| of.get(name).and_then(Value::as_str).unwrap_or_default();
	vec![[part("run"), part("file"), part("capturedAt"), part("url")].join(" ")]
}

/// How a page's `item` is named: `<run> <file>`.
#[must_use]
pub fn id_of(item: &str) -> String {
	item.splitn(3, ' ').take(2).collect::<Vec<_>>().join(" ")
}

/// A moment as an instant, to the millisecond, as `Date.parse` reads it.
fn instant(moment: &str) -> Option<i64> {
	chrono::DateTime::parse_from_rfc3339(moment).ok().map(|at| at.timestamp_millis())
}

/// Whether two records are of one address and one moment.
fn alike(held: &Value, url: &str, moment: &str) -> bool {
	let field = |name| held.get(name).and_then(Value::as_str);
	field("url") == Some(url) && field("capturedAt").and_then(instant).is_some_and(|at| Some(at) == instant(moment))
}

/// One page, `<run> <file> <capturedAt> <url>`, as the module doc says.
pub(super) async fn hold(_conn: &mut SqliteConnection, sources: &Sources, item: &str) -> Result<Held, Failed> {
	let mut parts = item.splitn(4, ' ');
	let (run, file) = (parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
	let (moment, url) = (parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
	if !crate::snips::is_run(run) || !PAGE_FILE.is_match(file) {
		return Ok(Held::Refused(Why::Missing));
	}
	let Some(folder) = sources.pages.as_deref() else {
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
	let pages = record.get("pages").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
	let image_of = |page: &Value, field: &str| page.get("image").and_then(|image| image.get(field)).and_then(Value::as_str).map(String::from);
	let Some(page) = pages.iter().find(|page| image_of(page, "name").as_deref() == Some(file)) else {
		return Ok(Held::Refused(Why::Gone));
	};
	if !alike(page, url, moment) {
		return Ok(Held::Refused(Why::Gone));
	}
	// `file` is a plain file name (`PAGE_FILE`): it reads only inside the bundle.
	let Some(bytes) = super::read_held(&bundle.join(file)).await? else {
		return Ok(Held::Refused(Why::Missing));
	};
	if Some(super::sha256_hex(&bytes)) != image_of(page, "sha256") {
		return Ok(Held::Refused(Why::Missing));
	}
	Ok(Held::Record {
		bundle: String::from(run),
		record: page.clone(),
		picture: (String::from(file), bytes),
	})
}

/// Whether home still holds a page: always. A page is no one's record, and
/// home never takes an essay back for one.
#[allow(clippy::unused_async)]
pub(super) async fn still_held(_conn: &mut SqliteConnection, _sources: &Sources, _item: &str) -> Result<bool, sqlx::Error> {
	Ok(true)
}

/// `pages.json`, one record per file in the order first named, then each
/// picture; none for none. `Err` with each `<run> <file>` whose file another
/// run gives unlike: another address, moment or picture.
pub(super) fn given(paged: Vec<(String, Value, File)>) -> Result<Vec<File>, Vec<String>> {
	if paged.is_empty() {
		return Ok(Vec::new());
	}
	let mut kept: Vec<(String, Value, File)> = Vec::new();
	let mut unlike: Vec<String> = Vec::new();
	for (run, record, picture) in paged {
		let Some((first, held, (_, bytes))) = kept.iter().find(|(_, _, (name, _))| *name == picture.0) else {
			kept.push((run, record, picture));
			continue;
		};
		let field = |name| record.get(name).and_then(Value::as_str).unwrap_or_default();
		if !alike(held, field("url"), field("capturedAt")) || *bytes != picture.1 {
			for id in [first.clone() + " " + &picture.0, run + " " + &picture.0] {
				if !unlike.contains(&id) {
					unlike.push(id);
				}
			}
		}
	}
	if !unlike.is_empty() {
		return Err(unlike);
	}
	let (records, pictures): (Vec<Value>, Vec<File>) = kept.into_iter().map(|(_, record, picture)| (record, picture)).unzip();
	let body = serde_json::json!({ "schema": SCHEMA, "pages": records });
	Ok(std::iter::once((String::from(RECORD), body.to_string().into_bytes())).chain(pictures).collect())
}
