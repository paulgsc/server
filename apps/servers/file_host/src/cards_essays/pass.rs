//! One pass of the cards essays.
//!
//! It reads `paulgsc/testing`'s `cards/index.json`, fetches and keeps each
//! essay it lists that home does not hold, then writes each essay home holds
//! that the render worker's folder lacks.
//!
//! - **Only what is missing.** A pass is one request for the index, plus one
//!   per listed essay home holds no essay of that key of. An essay home holds
//!   is never fetched again.
//! - **No index yet is no essays.** Until the first essay merges,
//!   `paulgsc/testing` has no `cards/index.json`: a `404` for it is a pass
//!   with nothing to take, not a failure, so a fresh home does not alert.
//! - **Checked before kept.** The index must read as
//!   `some-ui.cards-index/1`, each essay `{"key": …}` with a key
//!   [`is_key`] takes, `<slug>-<YYYY-MM-DD>`. An entry that isn't one (a day
//!   that is no calendar day, say, which `cards-check`'s pattern lets
//!   through) is skipped and counted, and the others are still taken. An essay's file must be a
//!   cut list (`some-ui.cut-list/…`) whose stamp's day is its key's, and be
//!   at most [`MAX_BODY`] bytes. Its full shape is the renderer's own
//!   (`@some-ui/video-render`'s `parseCardsEssay`), held by the routine's
//!   `cards-check` before the essay merged and again by `record:cards`
//!   before anything is drawn; home keeps the bytes as served.
//! - **Counts and kinds, never bytes.** What went wrong is a kind
//!   ([`Pass::failures`]): no URL, body or essay's words reaches a log.
//! - **No network in tests.** Fetching is `game_feed`'s [`Fetch`] port.
//! - **A spoken card's takes** (WALK7, paulgsc/server#1052,
//!   paulgsc/some-ui#3282): an essay whose shots hold a spoken card
//!   (`spoken.role == "spoken"`) is given only with the takes its exchanges
//!   name ([`spoken_takes`]), the mailbox subject's notes by id, whatever they
//!   were said for. They are written whole to `<key>.takes/` before the
//!   essay's `<key>.json`, so the worker never sees the essay without them:
//!   for each take its audio `<take>.<webm|ogg|m4a>`, its word times
//!   `<take>.words.json` where home kept them, and its transcript
//!   `<take>.transcript.json`, the `some-ui.voice-transcript/1` body the
//!   mailbox files, verdict included, and nothing else: the one take
//!   hand-off every essay uses (`take_files::hand_off`, as `rust_essays`'
//!   takes are given). A take not heard yet holds the essay back until it is; one
//!   home does not hold (never sent, deleted, its recording let go) or did
//!   not hear (its verdict garbled, nothing said or failed) refuses it: a
//!   spoken card has no silence to play in a take's place (some-ui C812).
//!   A refused essay is a failed row on Studio › Renders, as the worker's
//!   own refusals are: `<key>.failed.log` (the reason in words and the takes
//!   by id, never what was said) and `<key>.refused` (`reason=take-missing`
//!   or `reason=take-unheard`) in [`super::STATUS_FOLDER`], both gone once
//!   the essay is given. Every pass looks again at an essay not given.
//! - **No copy of a take outlives its need** ([`tidy`], each minute, as
//!   `rust_essays::tidy`): once an essay's video is made its `<key>.takes/`
//!   goes, and an essay given with a take home no longer holds (deleted from
//!   My data, its recording let go, or its account gone) is withdrawn from
//!   the worker, its `<key>.json` and `<key>.takes/` removed, so the next
//!   pass refuses it (`take-missing`). A `.<key>.takes.partial` a stopped
//!   pass left goes too.

use crate::game_feed::pass::{fetch_capped, Answer, Fetch, NoAnswer};
use cards_essay_repo::{Essay, Kept};
use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use serde_json::Value;
use sqlx::SqlitePool;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Where the essays are: `paulgsc/testing`'s `cards/` folder on `main`,
/// public, served without a token.
pub const SOURCE: &str = "https://raw.githubusercontent.com/paulgsc/testing/main/cards/";

/// What `cards/index.json` is (`@some-ui/video-render`'s
/// `CARDS_INDEX_SCHEMA`).
pub const INDEX_SCHEMA: &str = "some-ui.cards-index/1";

/// What an essay's file's schema starts with: a cut list, of any version.
pub const CUT_LIST_SCHEMA: &str = "some-ui.cut-list/";

/// The most bytes home takes for the index or an essay: an essay is a few KB
/// (the two showcases are under 20 KB), and the mailbox's own cap is the same.
pub const MAX_BODY: usize = 1_048_576;

/// The longest key home takes, as `cards_essay`'s check does.
pub const MAX_KEY: usize = 128;

/// What a pass did: the essays it added, and each thing that went wrong, by
/// kind.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Pass {
	/// Essays stored this pass.
	pub added: usize,
	/// Listed essays home already held.
	pub held: usize,
	/// Essays written to the render worker's folder this pass.
	pub exported: usize,
	/// Essays held back: a take a spoken card names is not heard yet, or no
	/// subject holds takes.
	pub waiting: usize,
	/// Essays refused: a take a spoken card names is missing or unheard.
	pub refused: usize,
	/// Each failure, by kind: `timeout`, `connect`, `transport`, `refused`
	/// (a status other than 200), `too_big`, `unreadable`, and `export`
	/// for an essay that could not be written to the folder.
	pub failures: Vec<&'static str>,
}

/// Whether `text` is a calendar day written `YYYY-MM-DD`, and nothing else:
/// chrono's parser also takes a sign, a space or a one-digit field, so the
/// day must print back as `text`.
fn is_day(text: &str) -> bool {
	NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok_and(|day| day.format("%Y-%m-%d").to_string() == text)
}

/// The day of an essay's key, when it is one.
///
/// A key is `<slug>-<YYYY-MM-DD>`: a slug of plain lowercase words joined by
/// `-` (`@some-ui/video-render`'s `CARDS_KEY`), then a calendar day, at most
/// [`MAX_KEY`] long. So a key is always one plain file name.
#[must_use]
pub fn day_of(key: &str) -> Option<&str> {
	if key.len() > MAX_KEY || !key.is_ascii() {
		return None;
	}
	let (slug, day) = key.split_at(key.len().checked_sub(10)?);
	let slug = slug.strip_suffix('-')?;
	let plain = |word: &str| !word.is_empty() && word.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit());
	(slug.split('-').all(plain) && is_day(day)).then_some(day)
}

/// Whether `key` is an essay's key ([`day_of`]).
#[must_use]
pub fn is_key(key: &str) -> bool {
	day_of(key).is_some()
}

/// What `cards/index.json` lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Index {
	/// Each entry that is `{"key": <an essay's key>}`.
	pub keys: Vec<String>,
	/// Entries that are not, each skipped.
	pub bad: usize,
}

/// What `cards/index.json` lists, or `None` when it does not read as
/// `some-ui.cards-index/1` (a schema, and an `essays` list).
#[must_use]
pub fn read_index(body: &[u8]) -> Option<Index> {
	let index: Value = serde_json::from_slice(body).ok()?;
	if index.get("schema")?.as_str()? != INDEX_SCHEMA {
		return None;
	}
	let mut read = Index { keys: Vec::new(), bad: 0 };
	for entry in index.get("essays")?.as_array()? {
		let key = entry
			.as_object()
			.filter(|entry| entry.len() == 1)
			.and_then(|entry| entry.get("key"))
			.and_then(Value::as_str)
			.filter(|key| is_key(key));
		match key {
			Some(key) => read.keys.push(key.to_owned()),
			None => read.bad += 1,
		}
	}
	Some(read)
}

/// Whether an essay's file reads as a cut list stamped on its key's day.
#[must_use]
pub fn is_essay(body: &[u8], key: &str) -> bool {
	let Ok(essay) = serde_json::from_slice::<Value>(body) else {
		return false;
	};
	let cut_list = essay.get("schema").and_then(Value::as_str).is_some_and(|schema| schema.starts_with(CUT_LIST_SCHEMA));
	let stamped = essay.get("stamp").and_then(|stamp| stamp.get("date")).and_then(Value::as_str);
	cut_list && stamped.is_some() && stamped == day_of(key)
}

/// Fetches `url` ([`fetch_capped`] at [`MAX_BODY`]), pushing a failure's
/// kind onto `pass`.
async fn fetched<F: Fetch>(fetch: &F, url: &str, pass: &mut Pass) -> Option<Vec<u8>> {
	fetch_capped(fetch, url, MAX_BODY).await.map_err(|kind| pass.failures.push(kind)).ok()
}

/// One pass: [`take_new`], then [`export`].
///
/// It takes each essay the repository lists that home lacks, then writes
/// each essay home holds that `folder` lacks, so the render worker sees
/// every essay, a restart included.
///
/// # Errors
/// A storage failure, which ends the pass.
pub async fn run_once<F: Fetch>(db: &SqlitePool, fetch: &F, given: &Given, clock: &(dyn Fn() -> DateTime<Utc> + Sync)) -> Result<Pass, sqlx::Error> {
	let mut pass = Pass::default();
	take_new(db, fetch, clock, &mut pass).await?;
	export(db, given, &mut pass).await?;
	Ok(pass)
}

/// Where a pass gives the render worker its essays, and whose takes a spoken
/// card plays.
#[derive(Debug, Clone)]
pub struct Given {
	/// The essays' folder ([`super::ESSAYS_FOLDER`]).
	pub essays: PathBuf,
	/// The worker's status folder ([`super::STATUS_FOLDER`]), where a refused
	/// essay is said.
	pub status: PathBuf,
	/// The mailbox's subject (`mailbox::pass_subject`), whose takes a spoken
	/// card plays; with none, an essay with a spoken card waits.
	pub subject: Option<String>,
	/// The worker's renders ([`super::RENDERS_FOLDER`]): a made video's
	/// essay keeps no takes.
	pub renders: PathBuf,
}

/// What [`tidy`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Tidied {
	/// Takes folders removed once their essay's video was made.
	pub takes_removed: usize,
	/// Essays taken back from the worker: a take they play is no longer held.
	pub withdrawn: usize,
	/// Files that could not be removed; tried again next minute.
	pub failed: usize,
}

/// Each minute, the takes given to the worker brought in line with what home holds.
///
/// As the module doc says: one directory read, then per takes folder one
/// `stat`, and while its video is not made, one read of its essay and one
/// indexed read per take.
///
/// # Errors
/// A storage failure.
pub async fn tidy(db: &SqlitePool, given: &Given) -> Result<Tidied, sqlx::Error> {
	let mut done = Tidied::default();
	let Ok(mut entries) = tokio::fs::read_dir(&given.essays).await else {
		return Ok(done);
	};
	let mut keys = Vec::new();
	while let Ok(Some(entry)) = entries.next_entry().await {
		let name = entry.file_name().to_string_lossy().into_owned();
		if let Some(key) = name.strip_suffix(".takes").filter(|key| is_key(key)) {
			keys.push(key.to_owned());
		} else if name.starts_with('.') && name.ends_with(".takes.partial") {
			// What a stopped pass left: the export and this run in one task,
			// one after the other, so none is being written now.
			done.failed += usize::from(remove_dir(&entry.path()).await.is_err());
		}
	}
	for key in keys {
		let takes = given.essays.join(String::from(&key) + ".takes");
		if given.renders.join(&key).join(super::VIDEO).exists() {
			done.takes_removed += 1;
			done.failed += usize::from(remove_dir(&takes).await.is_err());
			continue;
		}
		if !still_held(db, given.subject.as_deref(), &key).await? {
			done.withdrawn += 1;
			// The essay first, so the worker never finds it without its takes.
			let essay = given.essays.join(String::from(&key) + ".json");
			let gone = match tokio::fs::remove_file(&essay).await {
				Err(error) => error.kind() == std::io::ErrorKind::NotFound,
				Ok(()) => true,
			};
			done.failed += usize::from(!gone || remove_dir(&takes).await.is_err());
		}
	}
	Ok(done)
}

/// Whether `subject` still holds every take `key`'s essay plays.
async fn still_held(db: &SqlitePool, subject: Option<&str>, key: &str) -> Result<bool, sqlx::Error> {
	let Some(subject) = subject else {
		return Ok(false);
	};
	let mut conn = db.acquire().await?;
	let Some(body) = cards_essay_repo::body(&mut conn, key).await? else {
		return Ok(false);
	};
	for take in spoken_takes(&body) {
		// Held but not read back (an `Err` other than storage) is never a
		// reason to take it back.
		match voice_repo::note_head(&mut conn, subject, &take).await {
			Ok(None) => return Ok(false),
			Err(voice_repo::VoiceError::Storage(err)) => return Err(err),
			Ok(Some(_)) | Err(_) => {}
		}
	}
	Ok(true)
}

async fn remove_dir(dir: &Path) -> std::io::Result<()> {
	match tokio::fs::remove_dir_all(dir).await {
		Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
		_ => Ok(()),
	}
}

/// Reads the index, then fetches and keeps each essay it lists that home
/// holds none of, oldest day first. An essay that fails is left for the next
/// pass; the others are still taken.
async fn take_new<F: Fetch>(db: &SqlitePool, fetch: &F, clock: &(dyn Fn() -> DateTime<Utc> + Sync), pass: &mut Pass) -> Result<(), sqlx::Error> {
	let body = match fetch.get_at_most(&(String::from(SOURCE) + "index.json"), MAX_BODY).await {
		Ok(Answer { status: 200, body }) => body,
		// No essay has merged yet: nothing to take.
		Ok(Answer { status: 404, .. }) => return Ok(()),
		Ok(_) => {
			pass.failures.push("refused");
			return Ok(());
		}
		Err(NoAnswer(kind)) => {
			pass.failures.push(kind);
			return Ok(());
		}
	};
	let Some(Index { mut keys, bad }) = read_index(&body) else {
		pass.failures.push("unreadable");
		return Ok(());
	};
	pass.failures.extend(std::iter::repeat_n("unreadable", bad));
	let held: HashSet<String> = {
		let mut conn = db.acquire().await?;
		cards_essay_repo::held(&mut conn).await?.into_iter().map(|essay| essay.key).collect()
	};
	// By day, not by name: a slug-first key's name order isn't its day's.
	keys.sort_by(|a, b| (day_of(a), a).cmp(&(day_of(b), b)));
	keys.dedup();
	for key in keys {
		if held.contains(&key) {
			pass.held += 1;
			continue;
		}
		let Some(day) = day_of(&key) else { continue };
		// `key` is checked plain, so the URL names one file.
		let Some(body) = fetched(fetch, &(String::from(SOURCE) + &key + ".json"), pass).await else {
			continue;
		};
		if !is_essay(&body, &key) {
			pass.failures.push("unreadable");
			continue;
		}
		let taken_at = clock().to_rfc3339_opts(SecondsFormat::Millis, true);
		let essay = Essay {
			key: &key,
			day,
			body: &body,
			taken_at: &taken_at,
		};
		let mut conn = db.acquire().await?;
		match cards_essay_repo::keep(&mut conn, &essay).await? {
			Kept::Added => pass.added += 1,
			Kept::Held => pass.held += 1,
		}
	}
	Ok(())
}

/// Writes each essay home holds that the essays' folder has no file of, as
/// `<key>.json`, byte for byte, through a temporary name so the render
/// worker never reads half a file. An essay already written is left alone:
/// home never rewrites a kept essay, so neither does this. An essay with a
/// spoken card is written only with its takes, after them, and is held back
/// or refused as the module doc says.
async fn export(db: &SqlitePool, given: &Given, pass: &mut Pass) -> Result<(), sqlx::Error> {
	let folder = given.essays.as_path();
	let held = {
		let mut conn = db.acquire().await?;
		cards_essay_repo::held(&mut conn).await?
	};
	let missing: Vec<_> = held.into_iter().filter(|essay| !folder.join(String::from(&essay.key) + ".json").exists()).collect();
	if missing.is_empty() {
		return Ok(());
	}
	if tokio::fs::create_dir_all(folder).await.is_err() {
		pass.failures.push("export");
		return Ok(());
	}
	for essay in missing {
		let body = {
			let mut conn = db.acquire().await?;
			cards_essay_repo::body(&mut conn, &essay.key).await?
		};
		let Some(body) = body else { continue };
		let wanted = spoken_takes(&body);
		let takes = if wanted.is_empty() {
			None
		} else {
			match takes_of(db, given.subject.as_deref(), &wanted).await? {
				Takes::Ready(files) => Some(files),
				Takes::Waiting => {
					pass.waiting += 1;
					// A take that was missing and is now home waits to be
					// heard: no longer a refusal.
					clear_refused(&given.status, &essay.key).await;
					continue;
				}
				Takes::Unreadable => {
					pass.failures.push("unreadable");
					continue;
				}
				Takes::Refused(refusal) => {
					pass.refused += 1;
					if say_refused(&given.status, &essay.key, &refusal).await.is_err() {
						pass.failures.push("export");
					}
					continue;
				}
			}
		};
		if let Some(files) = &takes {
			if write_takes(folder, &essay.key, files).await.is_err() {
				pass.failures.push("export");
				continue;
			}
		}
		let file = folder.join(String::from(&essay.key) + ".json");
		let partial = folder.join(String::from(".") + &essay.key + ".json.partial");
		let written = tokio::fs::write(&partial, &body).await.is_ok() && tokio::fs::rename(&partial, &file).await.is_ok();
		if written {
			pass.exported += 1;
			if takes.is_some() {
				clear_refused(&given.status, &essay.key).await;
			}
		} else {
			pass.failures.push("export");
		}
	}
	Ok(())
}

/// Every take the essay `body`'s spoken cards name, each once.
///
/// In the order first named (`@some-ui/video-render`'s `spokenTakesOf`,
/// C817): each exchange of a shot whose `spoken.role` is `spoken` is
/// `{ ask, answer }`, two turns, each `{ voice: "kokoro", said }` or
/// `{ voice: "owner", take }`; every owner turn's `take`, the asking one's
/// included. None for an essay with no spoken card, or one that does not
/// read: the recorder refuses that one itself, as before.
#[must_use]
pub fn spoken_takes(body: &[u8]) -> Vec<String> {
	let Ok(essay) = serde_json::from_slice::<Value>(body) else {
		return Vec::new();
	};
	let mut takes: Vec<String> = Vec::new();
	let shots = essay.get("shots").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
	for spoken in shots.iter().filter_map(|shot| shot.get("spoken")) {
		if spoken.get("role").and_then(Value::as_str) != Some("spoken") {
			continue;
		}
		let exchanges = spoken.get("exchanges").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
		let turns = exchanges.iter().flat_map(|exchange| [exchange.get("ask"), exchange.get("answer")]).flatten();
		let owners = turns.filter(|turn| turn.get("voice").and_then(Value::as_str) == Some("owner"));
		for take in owners.filter_map(|turn| turn.get("take").and_then(Value::as_str)) {
			if !takes.iter().any(|held| held == take) {
				takes.push(take.to_owned());
			}
		}
	}
	takes
}

/// What home holds of the takes an essay's spoken cards name.
enum Takes {
	/// Each file of `<key>.takes/`, by name, and its bytes.
	Ready(Vec<(String, Vec<u8>)>),
	/// A take is not heard yet, or no subject holds takes: asked again next
	/// pass.
	Waiting,
	/// Never to be given as home holds its takes now.
	Refused(Refusal),
	/// A take home holds does not read back: this essay fails the pass, and
	/// the others are still given.
	Unreadable,
}

/// Why an essay was refused: one reason, and the takes it is for, by id.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Refusal {
	/// `take-missing` or `take-unheard`, Studio › Renders' `reason`.
	reason: &'static str,
	takes: Vec<String>,
}

/// The files of `wanted`'s takes, `subject`'s, or why not. A take missing
/// refuses before one unheard; a refusal outranks a wait, so an essay that
/// can never be given says so at once. A few indexed reads per take.
async fn takes_of(db: &SqlitePool, subject: Option<&str>, wanted: &[String]) -> Result<Takes, sqlx::Error> {
	let Some(subject) = subject else {
		return Ok(Takes::Waiting);
	};
	let mut conn = db.acquire().await?;
	let (mut missing, mut not_made_out, mut waiting) = (Vec::new(), Vec::new(), false);
	let mut files = Vec::with_capacity(wanted.len() * 3);
	for take in wanted {
		// A name the recorder would skip (`.`-led) or that is no note's id
		// is no take home could hand over.
		if take.starts_with('.') || !voice_repo::is_note_id(take) {
			missing.push(take.clone());
			continue;
		}
		let ask = crate::take_files::Ask {
			take,
			name: take,
			of: None,
			transcript: true,
		};
		let handed = match crate::take_files::hand_off(&mut conn, subject, ask).await {
			Ok(handed) => handed,
			Err(voice_repo::VoiceError::Storage(err)) => return Err(err),
			Err(_) => return Ok(Takes::Unreadable),
		};
		match handed {
			crate::take_files::Take::Files(handed) => files.extend(handed),
			crate::take_files::Take::Absent { .. } => missing.push(take.clone()),
			crate::take_files::Take::NotMadeOut => not_made_out.push(take.clone()),
			crate::take_files::Take::NotHeardYet => waiting = true,
		}
	}
	Ok(if !missing.is_empty() {
		Takes::Refused(Refusal {
			reason: "take-missing",
			takes: missing,
		})
	} else if !not_made_out.is_empty() {
		Takes::Refused(Refusal {
			reason: "take-unheard",
			takes: not_made_out,
		})
	} else if waiting {
		Takes::Waiting
	} else {
		Takes::Ready(files)
	})
}

/// Writes `files` into `<key>.takes/` in `folder`, whole: made in a
/// temporary folder and renamed into place, replacing what a stopped pass
/// left.
async fn write_takes(folder: &Path, key: &str, files: &[(String, Vec<u8>)]) -> std::io::Result<()> {
	let partial = folder.join(String::from(".") + key + ".takes.partial");
	let into = folder.join(String::from(key) + ".takes");
	for left in [&partial, &into] {
		match tokio::fs::remove_dir_all(left).await {
			Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
			_ => {}
		}
	}
	tokio::fs::create_dir_all(&partial).await?;
	for (name, body) in files {
		tokio::fs::write(partial.join(name), body).await?;
	}
	tokio::fs::rename(&partial, &into).await
}

/// The refusal's `.refused`, as the worker writes its own (`reason=…`), and
/// its failed log's words: the reason and the takes by id, never what was
/// said in them.
fn refusal_files(refusal: &Refusal) -> (String, String) {
	let words = match refusal.reason {
		"take-missing" => "a spoken card names a take home doesn't hold (never sent, deleted, or its recording let go)",
		_ => "home couldn't make out a take a spoken card plays, or heard nothing said in it, and a spoken card has no silence to play in its place",
	};
	(
		String::from("reason=") + refusal.reason + "\n",
		String::from("home refused this essay before the render: ") + words + ": " + &refusal.takes.join(", ") + "\n",
	)
}

/// Says `key` was refused in `status`: its failed log first, then its
/// `.refused` (the worker drops a `.refused` with no failed log beside it),
/// each written whole, and neither rewritten while it says the same, so the
/// row's time is when the refusal was first said.
async fn say_refused(status: &Path, key: &str, refusal: &Refusal) -> std::io::Result<()> {
	let (refused, log) = refusal_files(refusal);
	tokio::fs::create_dir_all(status).await?;
	for (name, text) in [(String::from(key) + ".failed.log", log), (String::from(key) + ".refused", refused)] {
		let file = status.join(&name);
		if tokio::fs::read(&file).await.is_ok_and(|held| held == text.as_bytes()) {
			continue;
		}
		let partial = status.join(String::from(".") + &name + ".partial");
		tokio::fs::write(&partial, text).await?;
		tokio::fs::rename(&partial, &file).await?;
	}
	Ok(())
}

/// Removes a refusal [`say_refused`] left, once the essay is given: what the
/// worker then makes of it is its own to say.
async fn clear_refused(status: &Path, key: &str) {
	for name in [String::from(key) + ".refused", String::from(key) + ".failed.log"] {
		let _ = tokio::fs::remove_file(status.join(name)).await;
	}
}

#[cfg(test)]
mod tests {
	use super::*;
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
		fn answer(&self, name: &str, answer: Result<Answer, NoAnswer>) {
			self.answers.lock().unwrap().insert(String::from(SOURCE) + name, answer);
		}

		fn ok(&self, name: &str, body: &[u8]) {
			self.answer(name, Ok(Answer { status: 200, body: body.to_vec() }));
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

	/// The folders under `dir`, the status folder inside it, and no subject.
	fn given(dir: &Path) -> Given {
		Given {
			essays: dir.to_path_buf(),
			status: dir.join("status"),
			subject: None,
			renders: dir.join("renders"),
		}
	}

	fn clock() -> DateTime<Utc> {
		DateTime::parse_from_rfc3339("2026-11-14T09:00:00.000Z").unwrap().with_timezone(&Utc)
	}

	fn index(keys: &[&str]) -> Vec<u8> {
		let essays: Vec<Value> = keys.iter().map(|key| serde_json::json!({ "key": key })).collect();
		serde_json::json!({ "schema": INDEX_SCHEMA, "essays": essays }).to_string().into_bytes()
	}

	fn essay(day: &str) -> Vec<u8> {
		serde_json::json!({
			"schema": "some-ui.cut-list/2",
			"title": "A machine upgrades itself",
			"series": { "id": "cheat-sheet", "version": 1 },
			"stamp": { "sources": ["paulgsc/nixos@abc1234"], "date": day },
			"shots": [],
		})
		.to_string()
		.into_bytes()
	}

	#[test]
	fn a_key_is_a_plain_slug_and_a_calendar_day() {
		for key in ["nixos-upgrade-2026-11-14", "a-2026-11-14", "rust2-scratch-pad-2026-02-28"] {
			assert!(is_key(key), "{key}");
		}
		assert_eq!(day_of("nixos-upgrade-2026-11-14"), Some("2026-11-14"));
		for key in [
			"2026-11-14",
			"-2026-11-14",
			"nixos--upgrade-2026-11-14",
			"NixOS-2026-11-14",
			"nix_os-2026-11-14",
			"../etc-2026-11-14",
			"nixos-2026-02-30",
			"nixos-2026-1-14",
			"nixos-+2026-11-14",
			"nixös-2026-11-14",
			"",
		] {
			assert!(!is_key(key), "{key:?} is not a key");
		}
		assert!(!is_key(&(String::from("a").repeat(MAX_KEY) + "-2026-11-14")), "too long");
	}

	#[tokio::test]
	async fn each_listed_essay_home_lacks_is_fetched_once_by_day_and_kept_as_served() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let fake = Fake::default();
		fake.ok("index.json", &index(&["rust-scratchpad-2026-11-14", "nixos-upgrade-2026-11-14", "zfs-2026-11-02"]));
		for (key, day) in [
			("rust-scratchpad-2026-11-14", "2026-11-14"),
			("nixos-upgrade-2026-11-14", "2026-11-14"),
			("zfs-2026-11-02", "2026-11-02"),
		] {
			fake.ok(&(String::from(key) + ".json"), &essay(day));
		}

		let first = run_once(&pool, &fake, &given(dir.path()), &clock).await.unwrap();
		assert_eq!((first.added, first.held, first.exported, first.failures.len()), (3, 0, 3, 0));
		assert_eq!(
			fake.asked(),
			["index.json", "zfs-2026-11-02.json", "nixos-upgrade-2026-11-14.json", "rust-scratchpad-2026-11-14.json"],
			"oldest day first, by key within a day"
		);
		assert_eq!(std::fs::read(dir.path().join("zfs-2026-11-02.json")).unwrap(), essay("2026-11-02"), "byte for byte");

		let again = run_once(&pool, &fake, &given(dir.path()), &clock).await.unwrap();
		assert_eq!((again.added, again.held, again.exported), (0, 3, 0));
		assert_eq!(fake.asked(), ["index.json"], "an essay home holds is never fetched again");
	}

	#[tokio::test]
	async fn what_does_not_check_out_is_counted_by_kind_and_left_for_the_next_pass() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let fake = Fake::default();
		assert_eq!(run_once(&pool, &fake, &given(dir.path()), &clock).await.unwrap().failures, ["connect"]);
		fake.answer(
			"index.json",
			Ok(Answer {
				status: 404,
				body: b"404: Not Found".to_vec(),
			}),
		);
		let none_yet = run_once(&pool, &fake, &given(dir.path()), &clock).await.unwrap();
		assert_eq!(none_yet, Pass::default(), "no index yet is no essays, not a failure");
		fake.answer(
			"index.json",
			Ok(Answer {
				status: 503,
				body: b"busy".to_vec(),
			}),
		);
		assert_eq!(run_once(&pool, &fake, &given(dir.path()), &clock).await.unwrap().failures, ["refused"]);
		fake.ok("index.json", b"<html>busy</html>");
		assert_eq!(run_once(&pool, &fake, &given(dir.path()), &clock).await.unwrap().failures, ["unreadable"]);
		fake.ok("index.json", br#"{"schema":"some-ui.call-index/1","essays":[]}"#);
		assert_eq!(run_once(&pool, &fake, &given(dir.path()), &clock).await.unwrap().failures, ["unreadable"], "another index");
		fake.ok("index.json", br#"{"schema":"some-ui.cards-index/1"}"#);
		assert_eq!(run_once(&pool, &fake, &given(dir.path()), &clock).await.unwrap().failures, ["unreadable"], "no essays list");
		// An entry that isn't an essay's key is skipped and counted; the
		// others are still taken.
		fake.ok("zfs-2026-11-02.json", &essay("2026-11-02"));
		for key in ["../secrets-2026-11-14", "NixOS-2026-11-14", "nixos-2026-02-30"] {
			fake.ok("index.json", &index(&["zfs-2026-11-02", key]));
			let pass = run_once(&pool, &fake, &given(dir.path()), &clock).await.unwrap();
			assert_eq!(pass.failures, ["unreadable"], "{key:?} is not a key");
			assert!(
				fake
					.asked()
					.iter()
					.all(|asked| !asked.contains("secrets") && !asked.contains("NixOS") && !asked.contains("02-30")),
				"{key:?} is never fetched"
			);
		}
		fake.ok("index.json", br#"{"schema":"some-ui.cards-index/1","essays":[{"key":"zfs-2026-11-02","title":"x"}]}"#);
		assert_eq!(
			run_once(&pool, &fake, &given(dir.path()), &clock).await.unwrap().failures,
			["unreadable"],
			"an entry is its key alone"
		);
		assert_eq!(
			cards_essay_repo::held(&mut pool.acquire().await.unwrap()).await.unwrap().len(),
			1,
			"the good essay was taken beside the bad entries"
		);
		fake.asked();

		fake.ok("index.json", &index(&["d-2026-11-14", "c-2026-11-13", "b-2026-11-12", "a-2026-11-11", "e-2026-11-15"]));
		fake.ok("a-2026-11-11.json", &vec![b' '; MAX_BODY + 1]);
		fake.ok("b-2026-11-12.json", &essay("2026-11-11"));
		fake.answer("c-2026-11-13.json", Err(NoAnswer("timeout")));
		fake.ok("d-2026-11-14.json", &essay("2026-11-14"));
		fake.ok("e-2026-11-15.json", br#"{"schema":"some-ui.call-week/1","stamp":{"date":"2026-11-15"}}"#);
		let pass = run_once(&pool, &fake, &given(dir.path()), &clock).await.unwrap();
		assert_eq!(
			pass.failures,
			["too_big", "unreadable", "timeout", "unreadable"],
			"too big, stamped another day, no answer, not a cut list"
		);
		assert_eq!((pass.added, pass.exported), (1, 1), "the good essay is still taken");

		fake.ok("c-2026-11-13.json", &essay("2026-11-13"));
		fake.asked();
		let next = run_once(&pool, &fake, &given(dir.path()), &clock).await.unwrap();
		assert_eq!(next.added, 1);
		assert_eq!(
			fake.asked(),
			["index.json", "a-2026-11-11.json", "b-2026-11-12.json", "c-2026-11-13.json", "e-2026-11-15.json"],
			"each failed essay is asked again"
		);
	}

	/// The render worker's folder gets each essay home holds, byte for byte,
	/// once: an essay already there is never rewritten, and a pass whose index
	/// can't be read still writes what home holds.
	#[tokio::test]
	async fn every_essay_held_is_written_for_the_render_worker_once() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let fake = Fake::default();
		fake.ok("index.json", &index(&["zfs-2026-11-02"]));
		fake.ok("zfs-2026-11-02.json", &essay("2026-11-02"));
		assert_eq!(run_once(&pool, &fake, &given(dir.path()), &clock).await.unwrap().exported, 1);
		let file = dir.path().join("zfs-2026-11-02.json");

		std::fs::write(&file, b"touched by hand").unwrap();
		fake.answer("index.json", Err(NoAnswer("timeout")));
		let again = run_once(&pool, &fake, &given(dir.path()), &clock).await.unwrap();
		assert_eq!((again.exported, again.failures.as_slice()), (0, &["timeout"][..]));
		assert_eq!(std::fs::read(&file).unwrap(), b"touched by hand", "a written essay is left alone");

		std::fs::remove_file(&file).unwrap();
		assert_eq!(
			run_once(&pool, &fake, &given(dir.path()), &clock).await.unwrap().exported,
			1,
			"a missing file is written again"
		);
		assert_eq!(std::fs::read(&file).unwrap(), essay("2026-11-02"));
		let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|entry| entry.unwrap().file_name()).collect();
		assert_eq!(names, ["zfs-2026-11-02.json"], "no partial file is left");
	}

	const ME: &str = "subject-me";

	/// An essay stamped `day` whose one spoken card plays `takes`, and a
	/// scene that is no spoken card.
	fn spoken_essay(day: &str, takes: &[&str]) -> Vec<u8> {
		let exchanges: Vec<Value> = takes
			.iter()
			.map(|take| serde_json::json!({ "ask": { "voice": "kokoro", "said": "Who are you?" }, "answer": { "voice": "owner", "take": take } }))
			.collect();
		serde_json::json!({
			"schema": "some-ui.cut-list/2",
			"title": "Hello",
			"series": { "id": "cheat-sheet", "version": 3 },
			"stamp": { "sources": ["paulgsc/nixos@abc1234"], "date": day },
			"shots": [
				{ "id": "intro", "spoken": { "role": "intro", "said": "Hello." } },
				{ "id": "me", "spoken": { "role": "spoken", "category": "introduction", "exchanges": exchanges } },
			],
		})
		.to_string()
		.into_bytes()
	}

	/// `ME`'s note `note`, a Studio take in ogg, with its audio.
	async fn hold_take(pool: &SqlitePool, note: &str) {
		sqlx::query("INSERT OR IGNORE INTO account (subject_id, user_handle) VALUES (?1, ?1)")
			.bind(ME)
			.execute(pool)
			.await
			.unwrap();
		sqlx::query(
			"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES (?1, ?2, 'studio', 'intro:s1:1', '2026-11-20T10:00:00.000Z', 'audio/ogg', 4, 's', 'now')",
		)
		.bind(ME)
		.bind(note)
		.execute(pool)
		.await
		.unwrap();
		sqlx::query("INSERT INTO voice_note_audio (subject_id, note_id, body) VALUES (?1, ?2, x'4f676753')")
			.bind(ME)
			.bind(note)
			.execute(pool)
			.await
			.unwrap();
	}

	/// What home heard in `note`: `text`, its words, and `verdict` when given.
	async fn hear(pool: &SqlitePool, note: &str, text: &str, verdict: Option<&str>) {
		sqlx::query(
			"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, language, language_p, duration_ms, text, took_ms, heard_at)
			 VALUES (?1, ?2, 'm', 'e', 1, 'heard', 'en', 1.0, 900, ?3, 1, '2026-11-20T10:01:00.000Z')",
		)
		.bind(ME)
		.bind(note)
		.bind(text)
		.execute(pool)
		.await
		.unwrap();
		sqlx::query("INSERT INTO voice_transcript_word (subject_id, note_id, model, word_index, text, start_ms, end_ms) VALUES (?1, ?2, 'm', 0, 'hello', 0, 400)")
			.bind(ME)
			.bind(note)
			.execute(pool)
			.await
			.unwrap();
		if let Some(verdict) = verdict {
			sqlx::query(
				"INSERT INTO voice_transcript_verdict (subject_id, note_id, model, attempt, rule, verdict, reasons, speech_ms, judged_at)
				 VALUES (?1, ?2, 'm', 1, 'heard-1', ?3, '[]', CASE WHEN ?3 IN ('heard', 'garbled') THEN 800 END, 'now')",
			)
			.bind(ME)
			.bind(note)
			.bind(verdict)
			.execute(pool)
			.await
			.unwrap();
		}
	}

	fn names_in(dir: &Path) -> Vec<String> {
		let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect();
		names.sort();
		names
	}

	/// some-ui's own `spoken-intro` sample, byte for byte
	/// (`packages/video-render/src/sample/videos/spoken-intro/cut-list.json`
	/// at paulgsc/some-ui@928c141), so both repos read the same essay.
	const SPOKEN_INTRO: &[u8] = include_bytes!("testdata/spoken-intro.cut-list.json");

	/// The C817 shape, as some-ui writes it: an owner turn may ask or
	/// answer, and a Kokoro turn names no take.
	#[test]
	fn the_sample_spoken_cards_takes_are_its_owner_turns_asking_or_answering() {
		assert_eq!(spoken_takes(SPOKEN_INTRO), ["sample-take-1", "sample-take-2"]);
		let old = serde_json::json!({ "shots": [{ "spoken": { "role": "spoken", "exchanges": [{ "ask": "Who?", "take": "t" }] } }] });
		assert!(spoken_takes(old.to_string().as_bytes()).is_empty(), "the pre-C817 shape names no owner turn");
	}

	/// The sample essay itself is given with both its takes, the one the
	/// owner asks with included.
	#[tokio::test]
	async fn the_sample_essay_is_given_with_the_take_it_asks_with_and_the_one_it_answers_with() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let fake = Fake::default();
		fake.ok("index.json", &index(&["spoken-intro-2026-11-14"]));
		fake.ok("spoken-intro-2026-11-14.json", SPOKEN_INTRO);
		for take in ["sample-take-1", "sample-take-2"] {
			hold_take(&pool, take).await;
			hear(&pool, take, "A made-up answer.", Some("heard")).await;
		}
		let mut mine = given(dir.path());
		mine.subject = Some(String::from(ME));
		let done = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!((done.added, done.exported, done.failures.len()), (1, 1, 0));
		assert_eq!(std::fs::read(dir.path().join("spoken-intro-2026-11-14.json")).unwrap(), SPOKEN_INTRO);
		assert_eq!(
			names_in(&dir.path().join("spoken-intro-2026-11-14.takes")),
			[
				"sample-take-1.ogg",
				"sample-take-1.transcript.json",
				"sample-take-1.words.json",
				"sample-take-2.ogg",
				"sample-take-2.transcript.json",
				"sample-take-2.words.json"
			]
		);
	}

	#[test]
	fn a_spoken_cards_takes_are_named_once_in_order_and_nothing_else_is() {
		assert_eq!(spoken_takes(&spoken_essay("2026-11-20", &["b", "a", "b"])), ["b", "a"]);
		assert!(spoken_takes(&essay("2026-11-20")).is_empty(), "no spoken card, no takes");
		assert!(spoken_takes(b"not json").is_empty());
	}

	/// An essay with a spoken card waits until every take it names is heard,
	/// then is given after its takes: each take's audio, word times and
	/// transcript, verdict included, and nothing else.
	#[tokio::test]
	async fn a_spoken_card_is_given_its_takes_once_each_is_heard() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let fake = Fake::default();
		fake.ok("index.json", &index(&["hello-2026-11-20"]));
		fake.ok("hello-2026-11-20.json", &spoken_essay("2026-11-20", &["take-1", "take-2", "take-1"]));
		hold_take(&pool, "take-1").await;
		hold_take(&pool, "take-2").await;
		hear(&pool, "take-1", "I build things.", Some("heard")).await;

		let none = run_once(&pool, &fake, &given(dir.path()), &clock).await.unwrap();
		assert_eq!((none.exported, none.waiting), (0, 1), "with no subject, a spoken card waits");
		let mut mine = given(dir.path());
		mine.subject = Some(String::from(ME));
		let early = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!((early.exported, early.waiting, early.refused), (0, 1, 0), "take-2 is not heard yet");
		assert!(!dir.path().join("hello-2026-11-20.json").exists());

		hear(&pool, "take-2", "From home.", None).await;
		let done = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!((done.exported, done.waiting, done.failures.len()), (1, 0, 0));
		assert_eq!(names_in(dir.path()), ["hello-2026-11-20.json", "hello-2026-11-20.takes"], "no partial is left");
		let takes = dir.path().join("hello-2026-11-20.takes");
		assert_eq!(
			names_in(&takes),
			[
				"take-1.ogg",
				"take-1.transcript.json",
				"take-1.words.json",
				"take-2.ogg",
				"take-2.transcript.json",
				"take-2.words.json"
			]
		);
		assert_eq!(std::fs::read(takes.join("take-1.ogg")).unwrap(), b"OggS");
		let words: Value = serde_json::from_slice(&std::fs::read(takes.join("take-1.words.json")).unwrap()).unwrap();
		assert_eq!(words, serde_json::json!([{ "text": "hello", "startMs": 0, "endMs": 400 }]));
		let transcript: Value = serde_json::from_slice(&std::fs::read(takes.join("take-1.transcript.json")).unwrap()).unwrap();
		assert_eq!(
			(&transcript["schema"], &transcript["note"], &transcript["text"], &transcript["verdict"]["kind"]),
			(
				&serde_json::json!("some-ui.voice-transcript/1"),
				&serde_json::json!("take-1"),
				&serde_json::json!("I build things."),
				&serde_json::json!("heard")
			)
		);
		let legacy: Value = serde_json::from_slice(&std::fs::read(takes.join("take-2.transcript.json")).unwrap()).unwrap();
		assert_eq!(legacy["verdict"], Value::Null, "kept before verdicts were: none, which the recorder reads as heard");
		assert_eq!(run_once(&pool, &fake, &mine, &clock).await.unwrap().exported, 0, "given once");
	}

	/// A take a spoken card names that home doesn't hold, or couldn't hear,
	/// refuses the essay: a failed log and a `.refused` its Renders row reads,
	/// said once, and gone once the essay is given.
	#[tokio::test]
	async fn a_spoken_card_whose_take_is_missing_or_unheard_is_refused_and_said() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let fake = Fake::default();
		fake.ok("index.json", &index(&["gone-2026-11-19", "mumbled-2026-11-20"]));
		fake.ok("gone-2026-11-19.json", &spoken_essay("2026-11-19", &["nope", "take-3"]));
		fake.ok("mumbled-2026-11-20.json", &spoken_essay("2026-11-20", &["take-3", "take-4"]));
		hold_take(&pool, "take-3").await;
		hear(&pool, "take-3", "mm hm", Some("garbled")).await;
		hold_take(&pool, "take-4").await;
		let mut mine = given(dir.path());
		mine.subject = Some(String::from(ME));

		let done = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!((done.exported, done.refused, done.waiting), (0, 2, 0), "a refusal outranks a wait");
		let status = dir.path().join("status");
		assert_eq!(std::fs::read_to_string(status.join("gone-2026-11-19.refused")).unwrap(), "reason=take-missing\n");
		assert_eq!(std::fs::read_to_string(status.join("mumbled-2026-11-20.refused")).unwrap(), "reason=take-unheard\n");
		let log = std::fs::read_to_string(status.join("gone-2026-11-19.failed.log")).unwrap();
		assert!(log.ends_with(": nope\n"), "{log}");
		assert!(!log.contains("mm hm"), "never what was said");
		assert!(!dir.path().join("gone-2026-11-19.json").exists() && !dir.path().join("gone-2026-11-19.takes").exists());

		let said_at = std::fs::metadata(status.join("gone-2026-11-19.failed.log")).unwrap().modified().unwrap();
		run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!(
			std::fs::metadata(status.join("gone-2026-11-19.failed.log")).unwrap().modified().unwrap(),
			said_at,
			"said once, not every pass"
		);

		// A model that later hears take-3 clearly, and take-4 heard: given,
		// and its refusal goes.
		sqlx::query(
			"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, language, language_p, duration_ms, text, took_ms, heard_at)
			 VALUES (?1, 'take-3', 'm2', 'e', 1, 'heard', 'en', 1.0, 900, 'Clearly now.', 1, '2026-11-21T10:00:00.000Z')",
		)
		.bind(ME)
		.execute(&pool)
		.await
		.unwrap();
		hear(&pool, "take-4", "Yes.", Some("heard")).await;
		let later = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!((later.exported, later.refused), (1, 1));
		assert!(dir.path().join("mumbled-2026-11-20.json").exists());
		assert!(!status.join("mumbled-2026-11-20.refused").exists() && !status.join("mumbled-2026-11-20.failed.log").exists());
	}

	/// A given essay's takes go once its video is made; one whose take home
	/// no longer holds is taken back from the worker, and refused next pass.
	#[tokio::test]
	async fn no_copy_of_a_take_outlives_its_video_or_its_note() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let fake = Fake::default();
		fake.ok("index.json", &index(&["made-2026-11-19", "kept-2026-11-20"]));
		fake.ok("made-2026-11-19.json", &spoken_essay("2026-11-19", &["take-1"]));
		fake.ok("kept-2026-11-20.json", &spoken_essay("2026-11-20", &["take-2"]));
		for take in ["take-1", "take-2"] {
			hold_take(&pool, take).await;
			hear(&pool, take, "Hello there.", Some("heard")).await;
		}
		let mut mine = given(dir.path());
		mine.subject = Some(String::from(ME));
		assert_eq!(run_once(&pool, &fake, &mine, &clock).await.unwrap().exported, 2);
		assert_eq!(tidy(&pool, &mine).await.unwrap(), Tidied::default(), "nothing to tidy yet");

		std::fs::create_dir_all(mine.renders.join("made-2026-11-19")).unwrap();
		std::fs::write(mine.renders.join("made-2026-11-19").join(crate::cards_essays::VIDEO), b"mp4").unwrap();
		sqlx::query("DELETE FROM voice_note_audio WHERE note_id = 'take-2'").execute(&pool).await.unwrap();
		assert_eq!(
			tidy(&pool, &mine).await.unwrap(),
			Tidied {
				takes_removed: 1,
				withdrawn: 1,
				failed: 0
			}
		);
		assert_eq!(names_in(dir.path()), ["made-2026-11-19.json", "renders"], "the made one keeps its essay alone");
		let next = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!((next.exported, next.refused), (0, 1), "the withdrawn one is refused, not given again");
		assert_eq!(
			std::fs::read_to_string(dir.path().join("status").join("kept-2026-11-20.refused")).unwrap(),
			"reason=take-missing\n"
		);
	}

	/// Each way a take is not made out refuses `take-unheard`, a deleted take
	/// `take-missing`; a take that was missing and is now home but not heard
	/// waits, and is no longer said as refused; a partial a stopped pass
	/// left goes.
	#[tokio::test]
	async fn every_kind_of_unheard_or_gone_take_is_said_and_a_wait_is_no_refusal() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let fake = Fake::default();
		let keys = ["silent-2026-11-16", "empty-2026-11-17", "failed-2026-11-18", "deleted-2026-11-19", "late-2026-11-20"];
		fake.ok("index.json", &index(&keys));
		for (key, take) in keys.iter().zip(["t-silent", "t-empty", "t-failed", "t-deleted", "t-late"]) {
			fake.ok(&(String::from(*key) + ".json"), &spoken_essay(&key[key.len() - 10..], &[take]));
		}
		hold_take(&pool, "t-silent").await;
		hear(&pool, "t-silent", "Hi.", Some("nothing_said")).await;
		hold_take(&pool, "t-empty").await;
		hear(&pool, "t-empty", " ", None).await;
		hold_take(&pool, "t-failed").await;
		sqlx::query(
			"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, took_ms, heard_at)
			 VALUES (?1, 't-failed', 'm', 'e', 1, 'failed', 1, 'then')",
		)
		.bind(ME)
		.execute(&pool)
		.await
		.unwrap();
		sqlx::query(
			"INSERT INTO voice_transcript_verdict (subject_id, note_id, model, attempt, rule, verdict, reasons, judged_at)
			 VALUES (?1, 't-failed', 'm', 1, 'heard-1', 'failed', '[\"engine\"]', 'now')",
		)
		.bind(ME)
		.execute(&pool)
		.await
		.unwrap();
		sqlx::query(
			"INSERT INTO voice_note_deletion (subject_id, note_id, of_kind, recorded_at, size, stored_at, deleted_at, went, stayed)
			 VALUES (?1, 't-deleted', 'studio', 'then', 4, 'then', 'now', '[]', '[]')",
		)
		.bind(ME)
		.execute(&pool)
		.await
		.unwrap();
		let mut mine = given(dir.path());
		mine.subject = Some(String::from(ME));
		std::fs::create_dir_all(dir.path().join(".gone-2026-11-01.takes.partial")).unwrap();
		std::fs::write(dir.path().join(".gone-2026-11-01.takes.partial").join("t.ogg"), b"OggS").unwrap();

		let done = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!((done.exported, done.refused, done.waiting), (0, 5, 0));
		let reason = |key: &str| std::fs::read_to_string(dir.path().join("status").join(String::from(key) + ".refused")).unwrap();
		for key in &keys[..3] {
			assert_eq!(reason(key), "reason=take-unheard\n", "{key}");
		}
		assert_eq!(reason("deleted-2026-11-19"), "reason=take-missing\n");
		assert_eq!(reason("late-2026-11-20"), "reason=take-missing\n", "not sent yet: missing");

		hold_take(&pool, "t-late").await;
		let later = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!((later.refused, later.waiting), (4, 1));
		let status = dir.path().join("status");
		assert!(
			!status.join("late-2026-11-20.refused").exists() && !status.join("late-2026-11-20.failed.log").exists(),
			"home now, waiting to be heard: no longer refused"
		);

		assert_eq!(tidy(&pool, &mine).await.unwrap(), Tidied::default());
		assert!(!dir.path().join(".gone-2026-11-01.takes.partial").exists(), "a stopped pass's partial goes");
	}

	/// What the render worker made reads as rendered, waiting or failed, as
	/// every job's.
	#[tokio::test]
	async fn what_the_worker_made_of_each_essay_is_counted() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let fake = Fake::default();
		fake.ok("index.json", &index(&["a-2026-11-11", "b-2026-11-12", "c-2026-11-13"]));
		for (key, day) in [("a-2026-11-11", "2026-11-11"), ("b-2026-11-12", "2026-11-12"), ("c-2026-11-13", "2026-11-13")] {
			fake.ok(&(String::from(key) + ".json"), &essay(day));
		}
		run_once(&pool, &fake, &given(&dir.path().join("essays")), &clock).await.unwrap();
		let (renders, status) = (dir.path().join("renders"), dir.path().join("status"));
		std::fs::create_dir_all(renders.join("a-2026-11-11")).unwrap();
		std::fs::write(renders.join("a-2026-11-11").join(crate::cards_essays::VIDEO), b"mp4").unwrap();
		std::fs::create_dir_all(&status).unwrap();
		std::fs::write(status.join("b-2026-11-12.failed.log"), b"refused").unwrap();
		std::fs::write(status.join("last-pass"), b"1763110800\n").unwrap();

		let seen = crate::cards_essays::observe(&pool, &renders, &status).await.unwrap();
		assert_eq!((seen.held, seen.waiting, seen.failed, seen.let_go), (3, 2, 1, 0));
		assert_eq!(seen.worker_last_pass, Some(1_763_110_800));
		assert_eq!(seen.oldest_waiting_taken, Some(clock().timestamp()));
	}

	/// paulgsc/some-ui#3339: a take every attempt failed on refuses its
	/// essay (`take-unheard`); once the owner asks home to hear it again, the
	/// essay waits for its fresh attempts instead of being refused.
	#[tokio::test]
	async fn a_take_home_hears_again_holds_its_essay_back_rather_than_refusing_it() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let fake = Fake::default();
		let key = "again-2026-11-21";
		fake.ok("index.json", &index(&[key]));
		fake.ok(&(String::from(key) + ".json"), &spoken_essay("2026-11-21", &["t-failed"]));
		hold_take(&pool, "t-failed").await;
		for statement in [
			"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, took_ms, heard_at)
			 VALUES (?1, 't-failed', 'm', 'e', 3, 'deadline', 1, 'then')",
			"INSERT INTO voice_transcript_verdict (subject_id, note_id, model, attempt, rule, verdict, reasons, judged_at)
			 VALUES (?1, 't-failed', 'm', 3, 'heard-1', 'failed', '[\"deadline\"]', 'now')",
		] {
			sqlx::query(statement).bind(ME).execute(&pool).await.unwrap();
		}
		let mut mine = given(dir.path());
		mine.subject = Some(String::from(ME));
		let done = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!((done.refused, done.waiting), (1, 0), "every attempt failed: refused");

		let mut conn = pool.acquire().await.unwrap();
		let keys = voice_repo::transcript::Keys {
			plain: "m",
			verbatim: "m+verbatim-1",
		};
		let acted = voice_repo::attention::hear_again(&mut conn, ME, "t-failed", keys, "2026-11-21T09:00:00.000Z")
			.await
			.unwrap();
		assert!(matches!(acted, voice_repo::attention::Acted::Recorded(_)), "{acted:?}");
		drop(conn);
		let later = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!((later.refused, later.waiting), (0, 1), "heard again: it waits");
		assert!(!dir.path().join("status").join(String::from(key) + ".refused").exists(), "no longer said refused");
	}
}
