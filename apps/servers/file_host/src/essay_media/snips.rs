//! Snips, one kind of [`super`]: screens of the app from the snip library
//! (SNIPS4, paulgsc/some-ui#3114).
//!
//! A cut list's picture `{"kind": "screen", "of": {"from": "snips", "id":
//! <id>, "commit": <40 hex>, "at": [...]}}` names one, as `<commit> <id>`
//! (C868): a snip is its id in its commit's bundle. Home hands each beside
//! the essay in `<key>.snips/`: its picture under its bundle name
//! (`web/phone/<slug>-<hash12>.png`) and `snips.json` ([`given`]), one entry
//! per commit the essay names with the records of its snips, as the
//! recorder's `--snips` takes them (some-ui's `docs/snips.md`).
//!
//! Home keeps no picture library (the owner, 2026-11-21). The pictures come
//! from the bundle `crate::snips` fetches for a waiting essay into its cache
//! ([`Sources::snips`]): while a commit's bundle is not there yet the essay
//! waits; a commit marked gone refuses it (`snip-gone`); an id its bundle
//! doesn't hold, or a picture that is not its record's, is missing
//! (`snip-missing`), as is every snip when home fetches none. A snip is no
//! one's record, so it never takes an essay back: its copy goes with the
//! video.

use super::{Failed, File, Held, Sources, Why};
use serde_json::Value;
use sqlx::SqliteConnection;
use std::path::Path;

/// What `snips.json` is.
pub const GIVEN_SCHEMA: &str = "some-ui.snips-given/1";

/// The snip a cut list's `shot` shows, as `<commit> <id>`.
#[must_use]
pub fn named(shot: &Value) -> Vec<String> {
	let Some(of) = super::screen_of(shot, "snips") else {
		return Vec::new();
	};
	let part = |name| of.get(name).and_then(Value::as_str).unwrap_or_default();
	vec![String::from(part("commit")) + " " + part("id")]
}

/// The record of `id` in the fetched bundle at `bundle`, or `None`.
async fn record_in(bundle: &Path, id: &str) -> Result<Option<Value>, Failed> {
	let Some(manifest) = super::read_held(&bundle.join("manifest.json")).await? else {
		return Ok(None);
	};
	let manifest: Value = serde_json::from_slice(&manifest).map_err(|_| Failed::Unreadable)?;
	let snips = manifest.get("snips").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
	Ok(snips.iter().find(|record| record.get("id").and_then(Value::as_str) == Some(id)).cloned())
}

/// One snip, `<commit> <id>`, as the module doc says.
pub(super) async fn hold(_conn: &mut SqliteConnection, sources: &Sources, item: &str) -> Result<Held, Failed> {
	let Some((commit, id)) = item.split_once(' ').filter(|(commit, id)| crate::snips::is_commit(commit) && crate::snips::is_snip_id(id)) else {
		return Ok(Held::Refused(Why::Missing));
	};
	let Some(folder) = sources.snips.as_deref() else {
		return Ok(Held::Refused(Why::Missing));
	};
	let bundle = crate::snips::bundle_of(folder, commit);
	if !bundle.is_dir() {
		return Ok(if crate::snips::gone_of(folder, commit).exists() {
			Held::Refused(Why::Gone)
		} else {
			Held::Waiting
		});
	}
	let Some(record) = record_in(&bundle, id).await? else {
		return Ok(Held::Refused(Why::Missing));
	};
	let image = record.get("image");
	let field = |field| image.and_then(|image| image.get(field)).and_then(Value::as_str).map(String::from).unwrap_or_default();
	let (name, sha256) = (field("name"), field("sha256"));
	// The bundle was checked whole when fetched; its names are plain
	// (`crate::snips`), and the folder writer takes no other.
	let Some(bytes) = super::read_held(&bundle.join(&name)).await? else {
		return Ok(Held::Refused(Why::Missing));
	};
	if name.is_empty() || super::sha256_hex(&bytes) != sha256 {
		return Ok(Held::Refused(Why::Missing));
	}
	Ok(Held::Snip {
		commit: String::from(commit),
		record,
		picture: (name, bytes),
	})
}

/// Whether home still holds a snip: always. A snip is no one's record, and
/// home never takes an essay back for one.
#[allow(clippy::unused_async)]
pub(super) async fn still_held(_conn: &mut SqliteConnection, _sources: &Sources, _item: &str) -> Result<bool, sqlx::Error> {
	Ok(true)
}

/// `snips.json`: for each commit, in the order first named, the records of
/// its snips the essay names.
pub(super) fn given(snipped: Vec<(String, Value)>) -> File {
	let mut bundles: Vec<(String, Vec<Value>)> = Vec::new();
	for (commit, record) in snipped {
		match bundles.iter_mut().find(|(held, _)| *held == commit) {
			Some((_, records)) => records.push(record),
			None => bundles.push((commit, vec![record])),
		}
	}
	let bundles: Vec<Value> = bundles.into_iter().map(|(commit, snips)| serde_json::json!({ "commit": commit, "snips": snips })).collect();
	let body = serde_json::json!({ "schema": GIVEN_SCHEMA, "bundles": bundles });
	(String::from("snips.json"), body.to_string().into_bytes())
}
