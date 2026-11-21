//! Clip frames, one kind of [`super`]: frames of the Clips Library's clips,
//! as home keeps them (`clip_frame`, `clips::FRAMES_FOLDER`).
//!
//! A cut list's picture `{"kind": "screen", "of": {"from": "clips", "clip":
//! <id>, "side": "before" | "after", "file": <name>.png}}` names one, as
//! `<clip>/<side>/<file>` (some-ui's `clipFramePath`). Home hands each beside
//! the essay under that same path in `<key>.clips/`, as the recorder's
//! `--clips` takes them: the frames alone, no list.
//!
//! A frame is handed only when `clip_frame` holds it and the PNG on the drive
//! is the one its row names; any other is missing (`clip-missing`). This
//! only reads: a `clip_frame` row is never written here (CL2).

use super::{Failed, Held, Sources, Why};
use serde_json::Value;
use sqlx::SqliteConnection;

/// The frame a cut list's `shot` shows, as `<clip>/<side>/<file>`.
#[must_use]
pub fn named(shot: &Value) -> Vec<String> {
	let Some(of) = super::screen_of(shot, "clips") else {
		return Vec::new();
	};
	let part = |name| of.get(name).and_then(Value::as_str).unwrap_or_default();
	vec![[part("clip"), part("side"), part("file")].join("/")]
}

/// `item`'s clip, side and file, when each is one `clip_frame` could hold:
/// a clip id as some-ui names one (`^[a-z0-9][a-z0-9-]{0,63}$`), `before`
/// or `after`, and a plain `.png` name of at most 80 bytes, as `clip_frame`
/// holds them.
fn parts(item: &str) -> Option<(&str, &str, &str)> {
	let mut parts = item.split('/');
	let (clip, side, file) = (parts.next()?, parts.next()?, parts.next()?);
	let plain_clip = (1..=64).contains(&clip.len())
		&& clip.as_bytes().first().is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
		&& clip.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
	let plain_file = (5..=80).contains(&file.len())
		&& file.rsplit_once('.').is_some_and(|(_, extension)| extension == "png")
		&& file.as_bytes().first().is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
		&& file.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'_' || b == b'-');
	(parts.next().is_none() && plain_clip && matches!(side, "before" | "after") && plain_file).then_some((clip, side, file))
}

/// One frame, by `<clip>/<side>/<file>`: its PNG when home holds it as its
/// row says, else missing. A row and a file, never a wait: a frame is kept
/// whole or not at all.
pub(super) async fn hold(conn: &mut SqliteConnection, sources: &Sources, item: &str) -> Result<Held, Failed> {
	let Some((clip, side, file)) = parts(item) else {
		return Ok(Held::Refused(Why::Missing));
	};
	let Some(row) = clip_frame_repo::one(conn, clip, side, file).await? else {
		return Ok(Held::Refused(Why::Missing));
	};
	let Some(bytes) = super::read_held(&sources.frames.join(clip).join(side).join(file)).await? else {
		return Ok(Held::Refused(Why::Missing));
	};
	if super::sha256_hex(&bytes) != row.sha256 {
		return Ok(Held::Refused(Why::Missing));
	}
	Ok(Held::Files(vec![(String::from(item), bytes)]))
}

/// Whether home still holds `item`'s frame: its row, and its PNG on the
/// drive. A drive that cannot say is no reason to take an essay back.
pub(super) async fn still_held(conn: &mut SqliteConnection, sources: &Sources, item: &str) -> Result<bool, sqlx::Error> {
	let Some((clip, side, file)) = parts(item) else {
		return Ok(false);
	};
	let on_drive = sources.frames.join(clip).join(side).join(file).try_exists().unwrap_or(true);
	Ok(clip_frame_repo::one(conn, clip, side, file).await?.is_some() && on_drive)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn only_a_name_clip_frame_could_hold_is_a_frame() {
		assert_eq!(parts("studio-promo/after/01.png"), Some(("studio-promo", "after", "01.png")));
		for bad in [
			"-promo/after/01.png",
			"studio-promo/middle/01.png",
			"../after/01.png",
			"studio-promo/after/../01.png",
			"studio-promo/after/01.jpg",
			"a/before/01.png/x",
			"a/before",
			"A/before/01.png",
		] {
			assert_eq!(parts(bad), None, "{bad}");
		}
	}
}
