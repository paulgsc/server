//! Snips, one kind of [`super`]: screens of the app from the snip library
//! (SNIPS4, paulgsc/some-ui#3114).
//!
//! A cut list's picture `{"kind": "screen", "of": {"from": "snips", "id":
//! <id>, "at": [...]}}` names one. Home hands each beside the essay in
//! `<key>.snips/`: its picture under its bundle name
//! (`web/phone/<slug>-<hash12>.png`) and `snips.json`, the records of the
//! snips the essay names (`some-ui.snips-given/1`, some-ui's `docs/snips.md`),
//! as the recorder's `--snips` takes them.
//!
//! Home keeps no picture library (the owner, 2026-11-21: nothing a CI run can
//! make again is kept twice). A snip comes from its capture's bundle, fetched
//! for the essay that names it (SNIPS2, paulgsc/some-ui#3116), which is not
//! built yet. Until it is, home holds no snip, and an essay that shows one is
//! refused and said (`snip-missing`), as some-ui's CE1 has it.

use super::{Failed, Held, Sources, Why};
use serde_json::Value;
use sqlx::SqliteConnection;

/// The snip a cut list's `shot` shows, by id.
#[must_use]
pub fn named(shot: &Value) -> Vec<String> {
	super::screen_of(shot, "snips")
		.and_then(|of| of.get("id").and_then(Value::as_str))
		.map(|id| vec![String::from(id)])
		.unwrap_or_default()
}

/// One snip, by id. Home holds none yet (the module doc): every one is
/// missing.
#[allow(clippy::unused_async)]
pub(super) async fn hold(_conn: &mut SqliteConnection, _sources: &Sources, _id: &str) -> Result<Held, Failed> {
	Ok(Held::Refused(Why::Missing))
}

/// Whether home still holds `id`'s snip. A snip is no one's record: home
/// never takes an essay back for one, and its copy goes with the video.
#[allow(clippy::unused_async)]
pub(super) async fn still_held(_conn: &mut SqliteConnection, _sources: &Sources, _id: &str) -> Result<bool, sqlx::Error> {
	Ok(true)
}
