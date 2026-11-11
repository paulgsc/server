//! One pass of the clips: read `paulgsc/testing`'s `clips/index.json`, then,
//! for each side of each clip, keep what home should hold of it and lacks.
//!
//! - **Only what is missing.** A pass is one request for the index, plus one
//!   per legacy frame home lacks and one per walk's sample not yet given to
//!   the render worker. A frame home holds is never fetched or read again.
//! - **Checked before kept.** The index must read as
//!   `some-ui.clip-index/1` or `/2`, each clip's id and each snip's file
//!   plain; a clip that doesn't read is counted and the others still taken.
//!   A frame must be a PNG by its magic bytes, at most [`MAX_FRAME`] bytes; a
//!   legacy frame must be its pinned bytes ([`LEGACY`]); a walk's `walk.json`
//!   must name the index's snips, in order, by file name and label, and a
//!   walk side has at most [`MAX_SNIPS`].
//! - **Append-only.** A frame's file is made once, never replaced: it is
//!   written beside its name and linked into place, which fails if the name
//!   is taken. A name already on the drive with other bytes, or held in
//!   `clip_frame` with other bytes, is `differs`, and nothing is written.
//! - **Counts and kinds, never bytes.** What went wrong is a kind
//!   ([`Pass::failures`]): no URL, body or label reaches a log.
//! - **No network in tests.** Fetching is `game_feed`'s [`Fetch`] port.

use crate::game_feed::pass::{fetch_capped, Fetch};
use chrono::{DateTime, SecondsFormat, Utc};
use clip_frame_repo::{Frame, Kept, Made};
use serde::Deserialize;
use sqlx::SqlitePool;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Where the clips are: `paulgsc/testing` on `main`, public, served without
/// a token. The index is `clips/index.json`; a frame or a sample is named by
/// its path from here.
pub const SOURCE: &str = "https://raw.githubusercontent.com/paulgsc/testing/main/";

/// What `clips/index.json` may be: the first, and the second, which adds a
/// walk side's `walk` and an optional `sha256` per snip.
pub const INDEX_SCHEMAS: [&str; 2] = ["some-ui.clip-index/1", "some-ui.clip-index/2"];

/// What a walk leaves beside its snips (`@some-ui/video-motion`'s `run.ts`).
pub const WALK_SCHEMA: &str = "some-ui.walk/1";

/// The most bytes home takes for the index, a sample, a `walk.json` or a
/// frame.
pub const MAX_FRAME: usize = 1_048_576;

/// The most snips a walk side may have.
pub const MAX_SNIPS: usize = 12;

/// The six frames taken by sessions before testing kept no media.
///
/// Each by its path in the repository and the SHA-256 it is held to: testing's
/// `scripts/guard.py`, `LEGACY_MEDIA`, copied. Home keeps these and no other
/// session-taken frame (the owner's decision, 2026-11-11).
pub const LEGACY: [(&str, &str); 6] = [
	(
		"clips/rust-essay-recorded/after/01-intro.png",
		"ef959543984d9fb4aa0390919c74a5b399e5703b5ee041c8aeaa74dad31a7847",
	),
	(
		"clips/rust-essay-recorded/after/02-answer.png",
		"d4a2b3d3b64d810fceb39f60dc0e35b9e3db5c89f6a6e40e9a95064fb3c6a55d",
	),
	(
		"clips/rust-essay-recorded/after/03-beside.png",
		"b71799b50440344e8e42133c1bb03481b77b12cae0a9284c0f9e69e71b19f730",
	),
	(
		"clips/studio-release-promo/after/01-templates.png",
		"bae1cfbaff2dcbfdc641e0cbc3ba69a95f3c9a265098c1e1c5e8fc1bb7d2475c",
	),
	(
		"clips/studio-release-promo/after/02-release-promo.png",
		"8c5b1ec2a8cd537beb3a97294aa66966ae97c31ed5478f69df66b7bfb5552692",
	),
	(
		"clips/studio-release-promo/before/01-templates.png",
		"4b09b718b7140852cb85a2ea35b3207a10293b6a2a769528b874704f87808701",
	),
];

/// A PNG's first eight bytes.
const PNG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// The folders a pass reads and writes: the server's (`clips`), or a
/// temporary one in tests.
#[derive(Debug, Clone)]
pub struct Folders {
	/// What home keeps: `<clip>/<side>/<file>`.
	pub frames: PathBuf,
	/// Each walk given to the render worker: `<clip>-<side>/{sample.json,app}`.
	pub walks: PathBuf,
	/// Each walk the render worker finished: `<clip>-<side>/walk.json` and
	/// its snips.
	pub walk_renders: PathBuf,
}

/// What a pass did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Pass {
	/// Frames kept this pass.
	pub kept: usize,
	/// Frames the index names that home already held.
	pub held: usize,
	/// Walks given to the render worker this pass.
	pub queued: usize,
	/// Each failure, by kind: `timeout`, `connect`, `transport`, `refused`
	/// (a status other than 200), `too_big`, `unreadable` (the index, a
	/// clip, a sample or a `walk.json`), `not_png`, `digest` (a legacy frame
	/// that isn't its pinned bytes), `too_many` (snips), `mismatch` (a walk
	/// that doesn't name the index's snips), `differs` (other bytes under a
	/// held name) and `drive` (a file that could not be read or written).
	pub failures: Vec<&'static str>,
}

#[derive(Debug, Deserialize)]
struct Index {
	schema: String,
	clips: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct Clip {
	id: String,
	before: Option<Side>,
	after: Side,
}

#[derive(Debug, Deserialize)]
struct Side {
	made: String,
	snips: Vec<Snip>,
	walk: Option<Walk>,
}

#[derive(Debug, Deserialize)]
struct Snip {
	file: String,
	label: String,
}

#[derive(Debug, Deserialize)]
struct Walk {
	app: String,
	sample: String,
}

#[derive(Debug, Deserialize)]
struct WalkFile {
	schema: String,
	snips: Vec<Snip>,
}

/// Whether `name` is a plain name: lowercase letters and digits in runs
/// joined by single dashes (`studio-release-promo`), at most 80 bytes.
fn is_plain(name: &str) -> bool {
	name.len() <= 80
		&& !name.is_empty()
		&& name
			.split('-')
			.all(|run| !run.is_empty() && run.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
}

/// Whether `path` is a plain relative path ending `.<extension>`: segments of
/// lowercase letters, digits, `.`, `_` and `-`, each starting with a letter
/// or digit, so never `..`, absolute or empty.
fn is_relative(path: &str, extension: &str) -> bool {
	path.len() <= 200
		&& path.ends_with(extension)
		&& path.split('/').all(|segment| {
			segment.as_bytes().first().is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
				&& segment.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'_' || b == b'-')
		})
}

/// A snip's file name, its last segment, as home keeps and serves it.
fn file_name(path: &str) -> &str {
	path.rsplit('/').next().unwrap_or(path)
}

/// The side's frames by the names home keeps them under, in order, when
/// every snip names a plain `.png` and no two share a name.
fn frame_names(side: &Side) -> Option<Vec<&str>> {
	let names: Vec<&str> = side.snips.iter().map(|snip| file_name(&snip.file)).collect();
	let plain = side.snips.iter().all(|snip| is_relative(&snip.file, ".png") && file_name(&snip.file).len() <= 80);
	let distinct = names.iter().collect::<HashSet<_>>().len() == names.len();
	(plain && distinct).then_some(names)
}

/// The key a walk side goes by in the render worker's folders.
#[must_use]
pub fn walk_key(clip: &str, side: &str) -> String {
	String::from(clip) + "-" + side
}

/// Whether `bytes` is a PNG home keeps: its magic first, at most
/// [`MAX_FRAME`].
#[must_use]
pub fn is_frame(bytes: &[u8]) -> bool {
	bytes.len() <= MAX_FRAME && bytes.starts_with(&PNG)
}

/// One pass: read the index, then take what each side needs.
///
/// # Errors
/// A storage failure, which ends the pass.
pub async fn run_once<F: Fetch>(db: &SqlitePool, fetch: &F, folders: &Folders, clock: &(dyn Fn() -> DateTime<Utc> + Sync)) -> Result<Pass, sqlx::Error> {
	run_pinned(db, fetch, folders, clock, &LEGACY).await
}

/// [`run_once`] with the legacy frames pinned by `pins`: [`LEGACY`]'s
/// digests are of the real PNGs, which no test holds.
async fn run_pinned<F: Fetch>(db: &SqlitePool, fetch: &F, folders: &Folders, clock: &(dyn Fn() -> DateTime<Utc> + Sync), pins: &[(&str, &str)]) -> Result<Pass, sqlx::Error> {
	let mut pass = Pass::default();
	let Some(body) = fetched(fetch, &(String::from(SOURCE) + "clips/index.json"), &mut pass).await else {
		return Ok(pass);
	};
	let Some(index) = serde_json::from_slice::<Index>(&body).ok().filter(|index| INDEX_SCHEMAS.contains(&index.schema.as_str())) else {
		pass.failures.push("unreadable");
		return Ok(pass);
	};
	let held: HashSet<(String, String, String)> = {
		let mut conn = db.acquire().await?;
		clip_frame_repo::held(&mut conn)
			.await?
			.into_iter()
			.map(|frame| (frame.clip_id, frame.side, frame.file))
			.collect()
	};
	for raw in index.clips {
		let Some(clip) = serde_json::from_value::<Clip>(raw).ok().filter(|clip| is_plain(&clip.id)) else {
			pass.failures.push("unreadable");
			continue;
		};
		for (name, side) in [("before", clip.before.as_ref()), ("after", Some(&clip.after))] {
			let Some(side) = side else { continue };
			let Some(names) = frame_names(side) else {
				pass.failures.push("unreadable");
				continue;
			};
			let lacking: Vec<&str> = names
				.iter()
				.copied()
				.filter(|file| !held.contains(&(clip.id.clone(), name.to_owned(), (*file).to_owned())))
				.collect();
			pass.held += names.len() - lacking.len();
			if lacking.is_empty() {
				continue;
			}
			let at = Taking {
				db,
				folders,
				clock,
				pins,
				clip: &clip.id,
				side: name,
			};
			match side.made.as_str() {
				"walk" => walk(fetch, &at, side, &mut pass).await?,
				"session" => legacy(fetch, &at, side, &lacking, &mut pass).await?,
				_ => pass.failures.push("unreadable"),
			}
		}
	}
	Ok(pass)
}

/// A side being taken.
struct Taking<'a> {
	db: &'a SqlitePool,
	folders: &'a Folders,
	clock: &'a (dyn Fn() -> DateTime<Utc> + Sync),
	/// The legacy frames, by path, and the SHA-256 each is held to.
	pins: &'a [(&'a str, &'a str)],
	clip: &'a str,
	side: &'a str,
}

/// Fetches `url` ([`fetch_capped`] at [`MAX_FRAME`]), pushing a failure's
/// kind onto `pass`.
async fn fetched<F: Fetch>(fetch: &F, url: &str, pass: &mut Pass) -> Option<Vec<u8>> {
	fetch_capped(fetch, url, MAX_FRAME).await.map_err(|kind| pass.failures.push(kind)).ok()
}

/// A session-taken side: each of its frames home lacks that [`LEGACY`]
/// names is fetched and kept if it is its pinned bytes; any other is not
/// home's to keep (its label is the index's).
async fn legacy<F: Fetch>(fetch: &F, at: &Taking<'_>, side: &Side, lacking: &[&str], pass: &mut Pass) -> Result<(), sqlx::Error> {
	for snip in &side.snips {
		if !lacking.contains(&file_name(&snip.file)) {
			continue;
		}
		let path = String::from("clips/") + at.clip + "/" + &snip.file;
		let Some((_, pinned)) = at.pins.iter().find(|(legacy, _)| *legacy == path) else {
			continue;
		};
		let Some(bytes) = fetched(fetch, &(String::from(SOURCE) + &path), pass).await else {
			continue;
		};
		if clip_frame_repo::sha256_hex(&bytes) != *pinned {
			pass.failures.push("digest");
			continue;
		}
		keep(at, file_name(&snip.file), Made::Legacy, &bytes, pass).await?;
	}
	Ok(())
}

/// A walk side: once the render worker has made it, its frames are checked
/// against the index and kept; until then, it is given to the worker.
async fn walk<F: Fetch>(fetch: &F, at: &Taking<'_>, side: &Side, pass: &mut Pass) -> Result<(), sqlx::Error> {
	if side.snips.len() > MAX_SNIPS {
		pass.failures.push("too_many");
		return Ok(());
	}
	let key = walk_key(at.clip, at.side);
	let made = at.folders.walk_renders.join(&key);
	match tokio::fs::read(made.join(crate::clips::WALK_FILE)).await {
		Ok(body) => take_walk(at, side, &made, &body, pass).await,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
			queue_walk(fetch, at.folders, at.clip, &key, side, pass).await;
			Ok(())
		}
		Err(_) => {
			pass.failures.push("drive");
			Ok(())
		}
	}
}

/// Gives the render worker a walk it has not been given: its sample, as the
/// repository serves it, and its app's name, made whole under a temporary
/// name and moved into place, so the worker never reads half of one. One
/// already given is left as it is.
async fn queue_walk<F: Fetch>(fetch: &F, folders: &Folders, clip: &str, key: &str, side: &Side, pass: &mut Pass) {
	let given = folders.walks.join(key);
	if tokio::fs::metadata(&given).await.is_ok() {
		return;
	}
	let Some(walk) = side.walk.as_ref().filter(|walk| is_plain(&walk.app) && is_relative(&walk.sample, ".json")) else {
		pass.failures.push("unreadable");
		return;
	};
	// The sample is named from the clip's own folder (`after/sample.json`),
	// a plain relative path, so it is always under `clips/<clip>/`.
	let Some(sample) = fetched(fetch, &(String::from(SOURCE) + "clips/" + clip + "/" + &walk.sample), pass).await else {
		return;
	};
	if serde_json::from_slice::<serde_json::Value>(&sample).is_err() {
		pass.failures.push("unreadable");
		return;
	}
	let partial = folders.walks.join(String::from(".") + key + ".partial");
	let _ = tokio::fs::remove_dir_all(&partial).await;
	let written = tokio::fs::create_dir_all(&partial).await.is_ok()
		&& tokio::fs::write(partial.join("sample.json"), &sample).await.is_ok()
		&& tokio::fs::write(partial.join("app"), String::from(&walk.app) + "\n").await.is_ok()
		&& tokio::fs::rename(&partial, &given).await.is_ok();
	if written {
		pass.queued += 1;
	} else {
		let _ = tokio::fs::remove_dir_all(&partial).await;
		pass.failures.push("drive");
	}
}

/// Keeps a finished walk's frames, once its `walk.json` names exactly the
/// index's snips, in order, by file name and label.
///
/// A walk that doesn't (`mismatch`), or whose `walk.json` or snips don't
/// read as a walk's, is rejected: nothing of it is kept, and its output is
/// removed, so the render worker walks it again (it skips any walk whose
/// `walk.json` is there). The walk the image draws is not the one the index
/// says until the render image moves with some-ui's `main`; removing the
/// rejected output is what lets a new image recover it, at the cost of one
/// short walk a pass, an hour apart, while it stays wrong. A frame already
/// kept is never touched.
async fn take_walk(at: &Taking<'_>, side: &Side, made: &Path, body: &[u8], pass: &mut Pass) -> Result<(), sqlx::Error> {
	match checked_walk(side, made, body).await {
		Ok(frames) => {
			for (file, bytes) in frames {
				keep(at, &file, Made::Walk, &bytes, pass).await?;
			}
		}
		Err(kind) => {
			pass.failures.push(kind);
			if kind != "drive" && tokio::fs::remove_dir_all(made).await.is_err() {
				pass.failures.push("drive");
			}
		}
	}
	Ok(())
}

/// A finished walk's frames, by the names home keeps them under, when the
/// walk is the index's and every snip is a frame home keeps; otherwise why
/// not. Every frame is read and checked before any is kept, so a side is
/// kept whole or not at all. `drive` is a read that failed, not a wrong
/// walk: it is tried again as it is.
async fn checked_walk(side: &Side, made: &Path, body: &[u8]) -> Result<Vec<(String, Vec<u8>)>, &'static str> {
	let walked = (body.len() <= MAX_FRAME)
		.then(|| serde_json::from_slice::<WalkFile>(body).ok())
		.flatten()
		.filter(|walked| walked.schema == WALK_SCHEMA && walked.snips.iter().all(|snip| is_relative(&snip.file, ".png")))
		.ok_or("unreadable")?;
	let same = walked.snips.len() == side.snips.len()
		&& walked
			.snips
			.iter()
			.zip(&side.snips)
			.all(|(walked, listed)| file_name(&walked.file) == file_name(&listed.file) && walked.label == listed.label);
	if !same {
		return Err("mismatch");
	}
	let mut frames = Vec::with_capacity(walked.snips.len());
	for snip in &walked.snips {
		let path = made.join(&snip.file);
		let length = tokio::fs::metadata(&path).await.map_err(|_| "drive")?.len();
		if length > MAX_FRAME as u64 {
			return Err("too_big");
		}
		let bytes = tokio::fs::read(&path).await.map_err(|_| "drive")?;
		if !is_frame(&bytes) {
			return Err("not_png");
		}
		frames.push((file_name(&snip.file).to_owned(), bytes));
	}
	Ok(frames)
}

/// Puts `bytes` on the drive as `<clip>/<side>/<file>` unless that name is
/// taken, then records it. A name taken by these very bytes (a pass that
/// stopped between the two) is recorded; by other bytes, `differs`.
async fn keep(at: &Taking<'_>, file: &str, made: Made, bytes: &[u8], pass: &mut Pass) -> Result<(), sqlx::Error> {
	if !is_frame(bytes) {
		pass.failures.push(if bytes.len() > MAX_FRAME { "too_big" } else { "not_png" });
		return Ok(());
	}
	let folder = at.folders.frames.join(at.clip).join(at.side);
	match put(&at.folders.frames, &folder, file, bytes).await {
		Put::Placed => {}
		Put::Differs => {
			pass.failures.push("differs");
			return Ok(());
		}
		Put::Failed => {
			pass.failures.push("drive");
			return Ok(());
		}
	}
	let taken_at = (at.clock)().to_rfc3339_opts(SecondsFormat::Millis, true);
	let frame = Frame {
		clip_id: at.clip,
		side: at.side,
		file,
		made,
		bytes,
		taken_at: &taken_at,
	};
	let mut conn = at.db.acquire().await?;
	match clip_frame_repo::keep(&mut conn, &frame).await? {
		Kept::Added => pass.kept += 1,
		Kept::Held => pass.held += 1,
		Kept::Differs => pass.failures.push("differs"),
	}
	Ok(())
}

/// What [`put`] did.
enum Put {
	/// The name now holds these bytes: made here, or already.
	Placed,
	/// The name holds other bytes; nothing was written.
	Differs,
	Failed,
}

/// Makes `folder/file` hold `bytes`, never replacing it: written whole
/// beside it and synced, then hard-linked into place, which fails if the
/// name is taken, so a frame already there is never overwritten; the folder
/// and the folders above it up to `root` are synced before `Placed`, so a
/// row is only ever written for a frame that is on the disk, name and bytes,
/// through a power loss.
///
/// One pass at a time: the clips worker is one task (`clips::start`), and a
/// second pass running `put` on the same name at once could link the other's
/// unfinished file.
async fn put(root: &Path, folder: &Path, file: &str, bytes: &[u8]) -> Put {
	let target = folder.join(file);
	let partial = folder.join(String::from(".") + file + ".partial");
	if tokio::fs::create_dir_all(folder).await.is_err() {
		return Put::Failed;
	}
	// A pass that stopped between the link and the removal below leaves the
	// partial as a second name of a kept frame: writing through it would
	// rewrite that frame. So it is unlinked first (the frame keeps its own
	// name), and the new one is made exclusively, a fresh file or nothing.
	match tokio::fs::remove_file(&partial).await {
		Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Put::Failed,
		_ => {}
	}
	if !write_new(&partial, bytes).await {
		let _ = tokio::fs::remove_file(&partial).await;
		return Put::Failed;
	}
	let linked = tokio::fs::hard_link(&partial, &target).await;
	let _ = tokio::fs::remove_file(&partial).await;
	let held = match linked {
		Ok(()) => true,
		// Already there (an earlier pass stopped before its row): these bytes
		// are read from memory, so they are synced before they count.
		Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => match tokio::fs::read(&target).await {
			Ok(there) if there == bytes => sync_file(&target).await,
			Ok(_) => return Put::Differs,
			Err(_) => false,
		},
		Err(_) => false,
	};
	// The name holds these bytes; it is `Placed` only once its entry is on
	// the disk too, and every folder's above it up to the frames root (a
	// clip's first frame made its clip's and side's folders just now).
	if held && sync_up(root, folder).await {
		Put::Placed
	} else {
		Put::Failed
	}
}

/// Writes `bytes` to `path` as a file made here, refusing one already there.
async fn write_new(path: &Path, bytes: &[u8]) -> bool {
	use tokio::io::AsyncWriteExt;
	let Ok(mut file) = tokio::fs::OpenOptions::new().write(true).create_new(true).open(path).await else {
		return false;
	};
	file.write_all(bytes).await.is_ok() && file.sync_all().await.is_ok()
}

/// Syncs `path`, a file or a folder (its entries), to the disk.
async fn sync_file(path: &Path) -> bool {
	match tokio::fs::File::open(path).await {
		Ok(file) => file.sync_all().await.is_ok(),
		Err(_) => false,
	}
}

/// Syncs `folder` and each folder above it up to `root`, both included, so
/// a name made in any of them survives a power loss.
async fn sync_up(root: &Path, folder: &Path) -> bool {
	let mut at = Some(folder);
	while let Some(dir) = at {
		if !sync_file(dir).await {
			return false;
		}
		if dir == root {
			return true;
		}
		at = dir.parent().filter(|parent| parent.starts_with(root));
	}
	false
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::game_feed::pass::{Answer, NoAnswer};
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
		fn ok(&self, name: &str, body: &[u8]) {
			self
				.answers
				.lock()
				.unwrap()
				.insert(String::from(SOURCE) + name, Ok(Answer { status: 200, body: body.to_vec() }));
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

	fn folders(root: &Path) -> Folders {
		Folders {
			frames: root.join("renders/clips"),
			walks: root.join("clip-walks"),
			walk_renders: root.join("renders/clip-walk"),
		}
	}

	fn clock() -> DateTime<Utc> {
		DateTime::parse_from_rfc3339("2026-11-11T09:00:00Z").unwrap().with_timezone(&Utc)
	}

	/// A PNG of `n` bytes past its magic.
	fn png(tail: &[u8]) -> Vec<u8> {
		[&PNG[..], tail].concat()
	}

	/// The first schema's index, with two of the legacy frames and one no
	/// pin names.
	const INDEX_1: &str = r#"{"schema":"some-ui.clip-index/1","clips":[
		{"id":"studio-release-promo","title":"t","app":"studio","change":{"repo":"some-ui","kind":"pr","number":2810},"landed":"2026-11-08",
		 "before":{"made":"session","snips":[{"file":"before/01-templates.png","label":"a"}]},
		 "after":{"made":"session","snips":[{"file":"after/01-templates.png","label":"b"},{"file":"after/09-not-pinned.png","label":"c"}]}}]}"#;

	/// The snip `i` (from 0) is named for: `01.png` and on.
	fn numbered(i: usize) -> String {
		let n = (i + 1).to_string();
		(if n.len() == 1 { String::from("0") + &n } else { n }) + ".png"
	}

	fn walk_index(labels: &[&str]) -> String {
		let snips: Vec<String> = labels
			.iter()
			.enumerate()
			.map(|(i, label)| String::from(r#"{"file":"after/snips/"#) + &numbered(i) + r#"","label":""# + label + r#"","sha256":""# + &"0".repeat(64) + r#""}"#)
			.collect();
		String::from(
			r#"{"schema":"some-ui.clip-index/2","clips":[{"id":"ledger-wake","title":"t","app":"suspender-ledger","change":{"repo":"some-ui","kind":"pr","number":1},"landed":"2026-11-11",
			"before":null,"after":{"made":"walk","walk":{"app":"suspender-ledger","sample":"after/sample.json"},"snips":["#,
		) + &snips.join(",")
			+ "]}}]}"
	}

	/// Writes what the render worker leaves for a walk: `walk.json` naming
	/// `labels`, and a snip for each.
	fn finish_walk(folders: &Folders, labels: &[&str]) {
		let made = folders.walk_renders.join("ledger-wake-after");
		std::fs::create_dir_all(made.join("snips")).unwrap();
		let snips: Vec<String> = labels
			.iter()
			.enumerate()
			.map(|(i, label)| {
				std::fs::write(made.join("snips").join(numbered(i)), png(label.as_bytes())).unwrap();
				String::from(r#"{"file":"snips/"#) + &numbered(i) + r#"","label":""# + label + r#"","atMs":"# + &(i * 1000).to_string() + "}"
			})
			.collect();
		let body = String::from(r#"{"schema":"some-ui.walk/1","app":"suspender-ledger","version":"1.0.0","snips":["#) + &snips.join(",") + r#"],"taps":[],"lengthMs":1}"#;
		std::fs::write(made.join("walk.json"), body).unwrap();
	}

	async fn rows(pool: &SqlitePool) -> Vec<(String, String, String)> {
		let mut conn = pool.acquire().await.unwrap();
		clip_frame_repo::held(&mut conn).await.unwrap().into_iter().map(|f| (f.clip_id, f.side, f.file)).collect()
	}

	#[tokio::test]
	async fn each_frame_home_lacks_is_kept_once_as_checked() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folders = folders(dir.path());
		let fake = Fake::default();
		fake.ok("clips/index.json", walk_index(&["Opens", "Keeps awake"]).as_bytes());
		fake.ok("clips/ledger-wake/after/sample.json", br#"{"schema":"some-ui.walk-sample/suspender-ledger/1"}"#);

		// First: nothing rendered yet, so the walk is given to the worker.
		let first = run_once(&pool, &fake, &folders, &clock).await.unwrap();
		assert_eq!((first.queued, first.kept, first.failures.clone()), (1, 0, vec![]));
		let given = folders.walks.join("ledger-wake-after");
		assert_eq!(std::fs::read_to_string(given.join("app")).unwrap(), "suspender-ledger\n");
		assert!(std::fs::read(given.join("sample.json")).unwrap().starts_with(b"{\"schema\""));
		assert_eq!(fake.asked(), ["clips/index.json", "clips/ledger-wake/after/sample.json"]);

		// Given once: the next pass asks for the index alone.
		let waiting = run_once(&pool, &fake, &folders, &clock).await.unwrap();
		assert_eq!((waiting.queued, waiting.failures.clone()), (0, vec![]));
		assert_eq!(fake.asked(), ["clips/index.json"]);

		// Rendered: its frames are kept, under their names, with a row each.
		finish_walk(&folders, &["Opens", "Keeps awake"]);
		let taken = run_once(&pool, &fake, &folders, &clock).await.unwrap();
		assert_eq!((taken.kept, taken.failures.clone()), (2, vec![]));
		let kept = folders.frames.join("ledger-wake/after");
		assert_eq!(std::fs::read(kept.join("01.png")).unwrap(), png(b"Opens"));
		assert_eq!(
			rows(&pool).await,
			[
				("ledger-wake".into(), "after".into(), "01.png".into()),
				("ledger-wake".into(), "after".into(), "02.png".into())
			]
		);

		// Held: nothing is read or written again, even if the drive changes.
		fake.asked();
		std::fs::write(folders.walk_renders.join("ledger-wake-after/snips/01.png"), png(b"redrawn")).unwrap();
		let again = run_once(&pool, &fake, &folders, &clock).await.unwrap();
		assert_eq!((again.kept, again.held, again.failures), (0, 2, vec![]));
		assert_eq!(std::fs::read(kept.join("01.png")).unwrap(), png(b"Opens"), "a held frame is never replaced");
		assert_eq!(fake.asked(), ["clips/index.json"]);
	}

	#[tokio::test]
	async fn a_walk_that_does_not_match_the_index_is_not_kept() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folders = folders(dir.path());
		let fake = Fake::default();
		fake.ok("clips/index.json", walk_index(&["Opens", "Keeps awake"]).as_bytes());
		finish_walk(&folders, &["Opens", "Suspends"]);
		let pass = run_once(&pool, &fake, &folders, &clock).await.unwrap();
		assert_eq!((pass.kept, pass.failures), (0, vec!["mismatch"]));
		assert!(rows(&pool).await.is_empty());
		assert!(!folders.frames.join("ledger-wake").exists(), "nothing of a mismatched walk is kept");
		assert!(
			!folders.walk_renders.join("ledger-wake-after").exists(),
			"its output is removed, so the worker walks it again (with a newer image, rightly)"
		);

		// A snip that is not a PNG keeps none of the side.
		finish_walk(&folders, &["Opens", "Keeps awake"]);
		std::fs::write(folders.walk_renders.join("ledger-wake-after/snips/02.png"), b"GIF89a").unwrap();
		let pass = run_once(&pool, &fake, &folders, &clock).await.unwrap();
		assert_eq!((pass.kept, pass.failures), (0, vec!["not_png"]));
		assert!(rows(&pool).await.is_empty());
		assert!(!folders.walk_renders.join("ledger-wake-after").exists());

		// Walked again rightly (a newer image): kept whole.
		finish_walk(&folders, &["Opens", "Keeps awake"]);
		let pass = run_once(&pool, &fake, &folders, &clock).await.unwrap();
		assert_eq!((pass.kept, pass.failures), (2, vec![]));

		// Past twelve snips, a walk side is refused before anything is asked.
		let many: Vec<String> = (0..13).map(|i| String::from("Step ") + &i.to_string()).collect();
		let many: Vec<&str> = many.iter().map(String::as_str).collect();
		fake.ok("clips/index.json", walk_index(&many).as_bytes());
		let pass = run_once(&pool, &fake, &folders, &clock).await.unwrap();
		assert_eq!(pass.failures, ["too_many"]);
	}

	#[tokio::test]
	async fn a_legacy_frame_is_held_to_its_pinned_digest() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folders = folders(dir.path());
		let fake = Fake::default();
		fake.ok("clips/index.json", INDEX_1.as_bytes());
		// Bytes that are not the pinned ones: refused, and asked again later.
		fake.ok("clips/studio-release-promo/before/01-templates.png", &png(b"edited"));
		fake.ok("clips/studio-release-promo/after/01-templates.png", &png(b"edited"));
		let pass = run_once(&pool, &fake, &folders, &clock).await.unwrap();
		assert_eq!((pass.kept, pass.failures), (0, vec!["digest", "digest"]));
		// A frame no pin names is not home's to keep: never asked for.
		assert_eq!(
			fake.asked(),
			[
				"clips/index.json",
				"clips/studio-release-promo/before/01-templates.png",
				"clips/studio-release-promo/after/01-templates.png"
			]
		);
		assert!(rows(&pool).await.is_empty());
		assert!(!folders.frames.exists());

		// Its pinned bytes (here, a test's own pin): kept, and never asked again.
		let pinned = png(b"the frame");
		let digest = clip_frame_repo::sha256_hex(&pinned);
		let pins = [("clips/studio-release-promo/after/01-templates.png", digest.as_str())];
		fake.ok("clips/studio-release-promo/after/01-templates.png", &pinned);
		let pass = run_pinned(&pool, &fake, &folders, &clock, &pins).await.unwrap();
		assert_eq!((pass.kept, pass.failures), (1, vec![]));
		assert_eq!(std::fs::read(folders.frames.join("studio-release-promo/after/01-templates.png")).unwrap(), pinned);
		assert_eq!(rows(&pool).await, [("studio-release-promo".into(), "after".into(), "01-templates.png".into())]);
		fake.asked();
		let again = run_pinned(&pool, &fake, &folders, &clock, &pins).await.unwrap();
		assert_eq!((again.kept, again.held, again.failures), (0, 1, vec![]));
		assert_eq!(fake.asked(), ["clips/index.json"]);
		// Every pin is a SHA-256 of a PNG path in a clip's folder.
		for (path, digest) in LEGACY {
			assert!(path.starts_with("clips/") && is_relative(path, ".png"), "{path}");
			assert!(digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()), "{digest}");
		}
	}

	#[tokio::test]
	async fn other_bytes_under_a_held_name_are_refused_and_never_written() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folders = folders(dir.path());
		let fake = Fake::default();
		fake.ok("clips/index.json", walk_index(&["Opens"]).as_bytes());
		// Something already under the name, not these bytes.
		let kept = folders.frames.join("ledger-wake/after");
		std::fs::create_dir_all(&kept).unwrap();
		std::fs::write(kept.join("01.png"), png(b"older")).unwrap();
		finish_walk(&folders, &["Opens"]);
		let pass = run_once(&pool, &fake, &folders, &clock).await.unwrap();
		assert_eq!((pass.kept, pass.failures), (0, vec!["differs"]));
		assert_eq!(std::fs::read(kept.join("01.png")).unwrap(), png(b"older"));
		assert!(rows(&pool).await.is_empty());

		// A pass that stopped after the link left its partial as a second name
		// of a kept frame: other bytes are refused, and the frame is untouched.
		std::fs::hard_link(kept.join("01.png"), kept.join(".01.png.partial")).unwrap();
		let pass = run_once(&pool, &fake, &folders, &clock).await.unwrap();
		assert_eq!((pass.kept, pass.failures), (0, vec!["differs"]));
		assert_eq!(std::fs::read(kept.join("01.png")).unwrap(), png(b"older"), "never rewritten through a leftover partial");
		assert!(!kept.join(".01.png.partial").exists());

		// The same bytes already there (a pass stopped before its row): kept.
		std::fs::remove_file(kept.join("01.png")).unwrap();
		std::fs::write(kept.join("01.png"), png(b"Opens")).unwrap();
		let pass = run_once(&pool, &fake, &folders, &clock).await.unwrap();
		assert_eq!((pass.kept, pass.failures), (1, vec![]));
		assert_eq!(rows(&pool).await.len(), 1);
	}

	#[tokio::test]
	async fn what_does_not_read_is_counted_and_the_rest_still_taken() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folders = folders(dir.path());
		let fake = Fake::default();
		// No answer: counted, nothing else asked.
		let pass = run_once(&pool, &fake, &folders, &clock).await.unwrap();
		assert_eq!(pass.failures, ["connect"]);
		fake.ok("clips/index.json", br#"{"schema":"some-ui.clip-index/9","clips":[]}"#);
		assert_eq!(run_once(&pool, &fake, &folders, &clock).await.unwrap().failures, ["unreadable"]);

		// A clip whose id or snip is not plain is counted; the walk beside it
		// is still given to the worker.
		let index = walk_index(&["Opens"]).replace(
			r#""clips":["#,
			r#""clips":[{"id":"../up","after":{"made":"session","snips":[]}},{"id":"bad-snip","after":{"made":"session","snips":[{"file":"../x.png","label":"x"}]}},"#,
		);
		fake.ok("clips/index.json", index.as_bytes());
		fake.ok("clips/ledger-wake/after/sample.json", b"{}");
		let pass = run_once(&pool, &fake, &folders, &clock).await.unwrap();
		assert_eq!((pass.queued, pass.failures), (1, vec!["unreadable", "unreadable"]));

		// A walk side with no walk to run (an index of the first schema) is
		// counted, never given.
		let no_walk = walk_index(&["Opens"])
			.replace(r#""walk":{"app":"suspender-ledger","sample":"after/sample.json"},"#, "")
			.replace("ledger-wake", "other-walk");
		fake.ok("clips/index.json", no_walk.as_bytes());
		let pass = run_once(&pool, &fake, &folders, &clock).await.unwrap();
		assert_eq!((pass.queued, pass.failures), (0, vec!["unreadable"]));
		assert!(!folders.walks.join("other-walk-after").exists());
	}
}
