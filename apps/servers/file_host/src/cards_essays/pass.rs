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
//! - **What an essay shows or plays is given with it** (`essay_media`, one
//!   family): a spoken card's takes (WALK7, paulgsc/server#1052,
//!   paulgsc/some-ui#3282), a scene's snips (SNIPS4) and clip frames. Each
//!   kind the essay names is written whole to `<key>.<kind>/` before the
//!   essay's `<key>.json`, so the worker never sees the essay without them.
//!   For takes, the mailbox subject's notes by id, whatever they were said
//!   for: each take's audio `<take>.<webm|ogg|m4a>`, its word times
//!   `<take>.words.json` where home kept them, and its transcript
//!   `<take>.transcript.json`, the `some-ui.voice-transcript/1` body the
//!   mailbox files, verdict included (`essay_media::takes::hand_off`, as
//!   `rust_essays`' takes are given). A take not heard yet holds the essay
//!   back until it is; an item home does not hold (a take never sent,
//!   deleted or let go; a snip or frame never kept) or a take it did not
//!   hear refuses it: a spoken card has no silence to play in a take's place
//!   (some-ui C812), and a scene is never a blank frame. A refused essay is a
//!   failed row on Studio › Renders, as the worker's own refusals are:
//!   `<key>.failed.log` (the reason in words and the items by id, never what
//!   they hold) and `<key>.refused` (`reason=take-missing`, `take-unheard`,
//!   `snip-missing`, `snip-gone` or `clip-missing`) in [`super::STATUS_FOLDER`], both gone
//!   once the essay is given. Every pass looks again at an essay not given.
//! - **No copy outlives its need** ([`tidy`], each minute, as
//!   `rust_essays::tidy`): once an essay's video is made its media folders
//!   go, and before then an essay given with an item home no longer holds
//!   otherwise (its account gone; a clip frame gone; never a snip, which is
//!   no one's record) is taken back from the worker, its `<key>.json` and
//!   media folders removed, so the next pass refuses it. A
//!   `.<key>.<kind>.partial` a stopped pass left goes too.
//! - **A take deleted withdraws its essay for good** (paulgsc/server#1055,
//!   as the Rust Rounds essay's: docs/identity.md's essay-render exception,
//!   rule (c)): an essay whose spoken cards play a take deleted from My data
//!   or let go is withdrawn whether its render has not started, is under way
//!   or is done, or it was never given (the pass, in place of a
//!   `take-missing` refusal): both receipts in `cards_essay_video_deletion`, cause
//!   `take-deleted`, naming the note beside the note's own receipt, then its
//!   `<key>.json`, takes, renders folder and work folder are removed. It is
//!   never given to the worker again, and no list shows it
//!   (`cards_essay_repo::gone`); a video a render under way finishes after
//!   that goes within a minute, as each minute's tidy removes whatever of a
//!   gone essay it finds. The take's own receipt stays in My data.

use crate::essay_media;
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
	/// Of those, the essays held back because a take is not heard yet:
	/// what waits on the transcriber (`crate::stage`, paulgsc/some-ui#3333).
	pub waiting_on_takes: usize,
	/// The takes those essays wait on, by essay key and note id: the edges
	/// `stage_want` keeps for the transcriber's queue (paulgsc/server#1091).
	pub wanted_takes: Vec<(String, String)>,
	/// Essays refused: an item they name is missing, unheard or gone
	/// (`essay_media::Refusal::reason`).
	pub refused: usize,
	/// Essays not given yet withdrawn for good: a take a spoken card names
	/// was deleted from My data or let go (paulgsc/server#1055).
	pub withdrawn: usize,
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
	export(db, given, clock, &mut pass).await?;
	if want(db, given, &pass).await.is_err() {
		// What the essays wait on orders the transcriber, nothing more: the
		// essays given stand, and the next pass says it again.
		pass.failures.push("want");
	}
	Ok(pass)
}

/// Says which takes the essays held back wait on (`stage_want`, the
/// transcriber's [`crate::stage::Waiter::CardsEssays`] edges), replacing
/// what the last pass said, so the transcriber hears them first
/// (paulgsc/server#1091). With no subject, nothing waits on a take. Under
/// account deletion's hold, and only for notes the subject still holds
/// (docs/identity.md invariant 9). A pass that failed before this leaves the
/// last pass's rows, which only order the transcriber and leave with the
/// account; one whose essays' folder could not be made says nothing waits.
async fn want(db: &SqlitePool, given: &Given, pass: &Pass) -> Result<(), sqlx::Error> {
	let subject = given.subject.as_deref().unwrap_or_default();
	let wants: Vec<_> = pass
		.wanted_takes
		.iter()
		.filter(|_| !subject.is_empty())
		.map(|(essay, take)| stage_repo::Want {
			waiter_key: essay,
			stage: crate::stage::Stage::Transcriber.name(),
			subject,
			item: take,
		})
		.collect();
	let _hold = given.deletion.hold().await;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	stage_repo::replace(&mut tx, crate::stage::Waiter::CardsEssays.name(), &wants).await?;
	tx.commit().await
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
	/// The clips' frames (`clips::FRAMES_FOLDER`), which a scene may show.
	pub frames: PathBuf,
	/// The snips bundles home fetches for waiting essays (`snips::FOLDER`),
	/// or `None` when it fetches none.
	pub snips: Option<PathBuf>,
	/// The worker's work folder ([`super::WORK_FOLDER`]): a withdrawn
	/// essay's waiting work goes with it.
	pub work: PathBuf,
	/// Account deletion's lock, held around writing what the essays wait on
	/// (docs/identity.md invariant 9).
	pub deletion: crate::auth::DeletionLock,
}

impl Given {
	/// Where the essays' media are read ([`essay_media::Sources`]).
	pub(crate) fn sources(&self) -> essay_media::Sources {
		essay_media::Sources {
			subject: self.subject.clone(),
			frames: self.frames.clone(),
			snips: self.snips.clone(),
		}
	}
}

/// What [`tidy`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Tidied {
	/// Media folders (of any kind; named for the first) removed once their
	/// essay's video was made.
	pub takes_removed: usize,
	/// Essays taken back from the worker before their video was made: an
	/// item they show or play is no longer held, or no one's takes are.
	pub taken_back: usize,
	/// Essays withdrawn for good: a take they play was deleted from My data
	/// or its recording let go, so their video and folder went.
	pub withdrawn: usize,
	/// Essays withdrawn already whose files were found and removed again: a
	/// render under way when its essay was withdrawn may still finish.
	pub removed: usize,
	/// Files that could not be removed; tried again next minute.
	pub failed: usize,
}

/// Each minute, what the worker was given and made brought in line with
/// what home holds (`essay_media`, "No copy outlives its need").
///
/// Under `feeds::FOLDERS`, as `rust_essays::tidy`, for each essay with a
/// file on the drive (its `<key>.json`, a media folder, or its renders or
/// work folder):
/// - an essay withdrawn (`cards_essay_repo::withdrawn_keys`) has all of
///   them removed, its status files too, and is tried again each minute
///   until it has, so a video a render under way finishes after its essay
///   was withdrawn goes within a minute too;
/// - an essay a take of whose was deleted from My data or let go
///   ([`deleted_take`]) is withdrawn for good, video made or not: both
///   receipts (`cards_essay_repo::withdraw`), then all of its files;
/// - once its video is made, its media folders go;
/// - before then, an essay given with an item home no longer holds, or with
///   no subject, is taken back (`<key>.json`, then its media folders).
///
/// Cost: three directory reads, then per essay on the drive one `stat`, one
/// indexed read of its body and, for each take its spoken cards name, one
/// indexed read of the take's receipt; and while its video is not made, a
/// few more per item it names.
///
/// # Errors
/// A storage failure.
pub async fn tidy(db: &SqlitePool, given: &Given, now: &str) -> Result<Tidied, sqlx::Error> {
	let _folders = crate::feeds::FOLDERS.lock().await;
	let mut done = Tidied::default();
	// Withdrawn only: a let-go (some-ui#2999) will go step by step, by its
	// own writer, never all at once here.
	let gone = {
		let mut conn = db.acquire().await?;
		cards_essay_repo::withdrawn_keys(&mut conn).await?
	};
	let mut keys = std::collections::BTreeSet::new();
	if let Ok(mut entries) = tokio::fs::read_dir(&given.essays).await {
		while let Ok(Some(entry)) = entries.next_entry().await {
			let name = entry.file_name().to_string_lossy().into_owned();
			if let Some((key, _)) = essay_media::parse_folder(&name).filter(|(key, _)| is_key(key)) {
				keys.insert(key.to_owned());
			} else if let Some(key) = name.strip_suffix(".json").filter(|key| is_key(key)) {
				keys.insert(key.to_owned());
			} else if let Some(key) = name.strip_prefix('.').and_then(|name| name.strip_suffix(".json.partial")).filter(|key| is_key(key)) {
				keys.insert(key.to_owned());
			} else if essay_media::is_partial(&name) {
				// What a stopped pass left: the export and this run in one task,
				// one after the other, so none is being written now.
				done.failed += usize::from(essay_media::remove_dir(&entry.path()).await.is_err());
			}
		}
	}
	for folder in [&given.renders, &given.work] {
		if let Ok(mut entries) = tokio::fs::read_dir(folder).await {
			while let Ok(Some(entry)) = entries.next_entry().await {
				let name = entry.file_name().to_string_lossy().into_owned();
				if is_key(&name) {
					keys.insert(name);
				}
			}
		}
	}
	let sources = given.sources();
	for key in keys {
		let essay = given.essays.join(String::from(&key) + ".json");
		let folders: Vec<PathBuf> = essay_media::Kind::ALL
			.into_iter()
			.map(|kind| essay_media::folder_of(&given.essays, &key, kind))
			.filter(|folder| folder.exists())
			.collect();
		let (render, work) = (given.renders.join(&key), given.work.join(&key));
		if gone.contains(&key) {
			done.removed += 1;
			done.failed += remove_all(&essay, &folders, &[&render, &work]).await;
			// What the worker said of it after, or before a crash let the
			// withdrawal clear it.
			clear_refused(&given.status, &key).await;
			continue;
		}
		if let Some(subject) = given.subject.as_deref() {
			if let Some(note) = deleted_take(db, subject, &key).await? {
				let size = i64::try_from(crate::call_weeks::render::bytes_under(&render).await).unwrap_or(i64::MAX);
				let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
				let withdrew = cards_essay_repo::withdraw(&mut tx, &key, subject, &note, size, now).await?;
				tx.commit().await?;
				// Not withdrawn here: gone already by another cause (a let-go,
				// some-ui#2999), whose own writer removes it step by step.
				if withdrew {
					done.withdrawn += 1;
					done.failed += remove_all(&essay, &folders, &[&render, &work]).await;
					clear_refused(&given.status, &key).await;
				}
				continue;
			}
		}
		if folders.is_empty() {
			continue;
		}
		if render.join(super::VIDEO).exists() {
			for folder in &folders {
				done.takes_removed += 1;
				done.failed += usize::from(essay_media::remove_dir(folder).await.is_err());
			}
			continue;
		}
		if !still_held(db, &sources, &key).await? {
			done.taken_back += 1;
			done.failed += remove_all(&essay, &folders, &[]).await;
		}
	}
	Ok(done)
}

/// Removes `essay` and any `.<essay>.partial` a stopped export left, then
/// its media `folders` and each of `dirs`; how many could not be removed.
/// The essay first, so the worker never finds it without its media: while
/// it stays, nothing else goes, and the next minute tries again.
async fn remove_all(essay: &Path, folders: &[PathBuf], dirs: &[&PathBuf]) -> usize {
	let mut failed = 0;
	let partial = essay.file_name().map(|name| essay.with_file_name(String::from(".") + &name.to_string_lossy() + ".partial"));
	for file in std::iter::once(essay.to_path_buf()).chain(partial) {
		let gone = match tokio::fs::remove_file(&file).await {
			Err(error) => error.kind() == std::io::ErrorKind::NotFound,
			Ok(()) => true,
		};
		failed += usize::from(!gone);
	}
	if failed > 0 {
		return failed;
	}
	for dir in folders.iter().chain(dirs.iter().copied()) {
		failed += usize::from(essay_media::remove_dir(dir).await.is_err());
	}
	failed
}

/// A take `key`'s spoken cards play that `subject` deleted from My data, or
/// whose recording was let go, if any: its receipt
/// (`voice_repo::delete::receipt`), kept for good, is the owner's act. A
/// cards essay is kept once and never rewritten, so what it plays is its one
/// body's, and a spoken card has no silence to play in a take's place (C812):
/// any such take withdraws it, as one the Rust Rounds essay plays does
/// (`rust_essays`' `deleted_take`).
async fn deleted_take(db: &SqlitePool, subject: &str, key: &str) -> Result<Option<String>, sqlx::Error> {
	let body = {
		let mut conn = db.acquire().await?;
		cards_essay_repo::body(&mut conn, key).await?
	};
	match body {
		Some(body) => deleted_of(db, subject, &spoken_takes(&body)).await,
		None => Ok(None),
	}
}

/// Every take the essay `body`'s spoken cards name, each once
/// (`essay_media`'s takes): the one rule this pass withdraws an essay by
/// ([`deleted_take`], and the export's own check) and `take_uses` shows the
/// phone (INUSE, C1382), so the badge and the delete never disagree.
pub(crate) fn spoken_takes(body: &[u8]) -> Vec<String> {
	essay_media::named(essay_media::Kind::Takes, &serde_json::from_slice(body).unwrap_or(Value::Null))
}

/// The first of `takes` that `subject` deleted or let go ([`deleted_take`]).
async fn deleted_of(db: &SqlitePool, subject: &str, takes: &[String]) -> Result<Option<String>, sqlx::Error> {
	let mut conn = db.acquire().await?;
	for take in takes {
		match voice_repo::delete::receipt(&mut conn, subject, take).await {
			Ok(None) => {}
			Err(voice_repo::VoiceError::Storage(err)) => return Err(err),
			// A receipt that does not read back is still a receipt.
			Ok(Some(_)) | Err(_) => return Ok(Some(take.clone())),
		}
	}
	Ok(None)
}

/// Whether home still holds every item `key`'s essay names, of every kind.
async fn still_held(db: &SqlitePool, sources: &essay_media::Sources, key: &str) -> Result<bool, sqlx::Error> {
	let mut conn = db.acquire().await?;
	let Some(body) = cards_essay_repo::body(&mut conn, key).await? else {
		return Ok(false);
	};
	let essay = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
	for kind in essay_media::Kind::ALL {
		if !essay_media::still_held(&mut conn, sources, kind, &essay).await? {
			return Ok(false);
		}
	}
	Ok(true)
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
/// home never rewrites a kept essay, so neither does this. An essay that
/// names media is written only after every kind of it (`essay_media`), and is
/// held back, refused or, for a take deleted, withdrawn, as the module doc
/// says. A withdrawn essay is never written.
async fn export(db: &SqlitePool, given: &Given, clock: &(dyn Fn() -> DateTime<Utc> + Sync), pass: &mut Pass) -> Result<(), sqlx::Error> {
	let folder = given.essays.as_path();
	let (held, gone) = {
		let mut conn = db.acquire().await?;
		(cards_essay_repo::held(&mut conn).await?, cards_essay_repo::gone(&mut conn).await?)
	};
	// A withdrawn essay is never given again.
	let missing: Vec<_> = held
		.into_iter()
		.filter(|essay| !gone.contains(&essay.key) && !folder.join(String::from(&essay.key) + ".json").exists())
		.collect();
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
		// Never given, or taken back before, and now a take it plays was
		// deleted: withdrawn for good, as one given would be by the tidy,
		// which removes what of it is left on the drive (a render taken back
		// mid-way, counted in its receipt's size). Asked before its media
		// are read, so another take that does not read back never holds it.
		let wanted = spoken_takes(&body);
		if let Some(subject) = given.subject.as_deref().filter(|_| !wanted.is_empty()) {
			if let Some(note) = deleted_of(db, subject, &wanted).await? {
				let now = clock().to_rfc3339_opts(SecondsFormat::Millis, true);
				// What a render taken back mid-way left, which the tidy removes.
				let size = i64::try_from(crate::call_weeks::render::bytes_under(&given.renders.join(&essay.key)).await).unwrap_or(i64::MAX);
				let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
				let withdrew = cards_essay_repo::withdraw(&mut tx, &essay.key, subject, &note, size, &now).await?;
				tx.commit().await?;
				pass.withdrawn += usize::from(withdrew);
				clear_refused(&given.status, &essay.key).await;
				continue;
			}
		}
		let named = {
			let essay = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
			let mut conn = db.acquire().await?;
			essay_media::for_essay(&mut conn, &given.sources(), &essay).await
		};
		let media = match named {
			Ok(essay_media::Given::Ready(media)) => media,
			Ok(essay_media::Given::Waiting(kinds)) => {
				pass.waiting += 1;
				// Waiting on the transcriber, not on there being no subject.
				let takes = kinds.into_iter().find(|(kind, _)| *kind == essay_media::Kind::Takes).map(|(_, takes)| takes);
				if let Some(takes) = takes.filter(|_| given.subject.is_some()) {
					pass.waiting_on_takes += 1;
					pass.wanted_takes.extend(takes.into_iter().map(|take| (essay.key.clone(), take)));
				}
				// An item that was missing and is now home waits (a take to be
				// heard): no longer a refusal.
				clear_refused(&given.status, &essay.key).await;
				continue;
			}
			Ok(essay_media::Given::Refused(refusal)) => {
				pass.refused += 1;
				if say_refused(&given.status, &essay.key, &refusal).await.is_err() {
					pass.failures.push("export");
				}
				continue;
			}
			Err(essay_media::Failed::Storage(err)) => return Err(err),
			Err(essay_media::Failed::Unreadable) => {
				pass.failures.push("unreadable");
				continue;
			}
		};
		let mut written = true;
		for (kind, files) in &media {
			written = written && essay_media::write(folder, &essay.key, *kind, files).await.is_ok();
		}
		if !written {
			// No folder of an essay not given is left: tried whole next pass.
			for (kind, _) in &media {
				let _ = essay_media::remove_dir(&essay_media::folder_of(folder, &essay.key, *kind)).await;
			}
			pass.failures.push("export");
			continue;
		}
		let file = folder.join(String::from(&essay.key) + ".json");
		let partial = folder.join(String::from(".") + &essay.key + ".json.partial");
		let written = tokio::fs::write(&partial, &body).await.is_ok() && tokio::fs::rename(&partial, &file).await.is_ok();
		if written {
			pass.exported += 1;
			if !media.is_empty() {
				clear_refused(&given.status, &essay.key).await;
			}
		} else {
			pass.failures.push("export");
		}
	}
	Ok(())
}

/// The most items a `.refused` names, as the worker's `REFUSED_SCENES`
/// (`feeds::renders::MOST_ITEMS`, which reads them): the failed log has
/// every one.
pub const REFUSED_ITEMS: usize = crate::feeds::renders::MOST_ITEMS;

/// The refusal's `.refused`, as the worker writes its own (`reason=…`), with
/// the first [`REFUSED_ITEMS`] items that block it by id and how many there
/// are (`items=`, `count=`), for Studio › Renders' row (`Run again` names
/// what still stops it); and its failed log's words: the reason and every
/// item by id, never what they hold.
fn refusal_files(refusal: &essay_media::Refusal) -> (String, String) {
	// Only ids of an item's shape, each at most 128 bytes, so the file stays
	// under the board's ceiling and reads whole; `count` is every item.
	let named: Vec<&str> = refusal
		.items
		.iter()
		.filter(|item| crate::feeds::renders::item_id(item))
		.take(REFUSED_ITEMS)
		.map(String::as_str)
		.collect();
	(
		String::from("reason=") + refusal.reason() + "\nitems=" + &named.join(",") + "\ncount=" + &refusal.items.len().to_string() + "\n",
		String::from("home refused this essay before the render: ") + refusal.words() + ": " + &refusal.items.join(", ") + "\n",
	)
}

/// Says `key` was refused in `status`: its failed log first, then its
/// `.refused` (the worker drops a `.refused` with no failed log beside it),
/// each written whole, and neither rewritten while it says the same, so the
/// row's time is when the refusal was first said.
async fn say_refused(status: &Path, key: &str, refusal: &essay_media::Refusal) -> std::io::Result<()> {
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
			frames: dir.join("frames-held"),
			snips: None,
			work: dir.join("work"),
			deletion: crate::auth::DeletionLock::default(),
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

	/// When a tidy runs.
	const NOW: &str = "2026-11-21T09:00:00.000Z";

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
		assert_eq!(
			(none.exported, none.waiting, none.waiting_on_takes),
			(0, 1, 0),
			"with no subject, a spoken card waits, and not on a take"
		);
		let mut mine = given(dir.path());
		mine.subject = Some(String::from(ME));
		let early = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!(
			(early.exported, early.waiting, early.waiting_on_takes, early.refused),
			(0, 1, 1, 0),
			"take-2 is not heard yet"
		);
		assert!(!dir.path().join("hello-2026-11-20.json").exists());
		let wants = || async {
			sqlx::query_as::<_, (String, String, String)>("SELECT waiter_key, subject_id, item_key FROM stage_want")
				.fetch_all(&pool)
				.await
				.unwrap()
		};
		let waiting_on = vec![(String::from("hello-2026-11-20"), String::from(ME), String::from("take-2"))];
		assert_eq!(wants().await, waiting_on, "the transcriber hears what the essay waits on first (#1091)");
		run_once(&pool, &fake, &given(dir.path()), &clock).await.unwrap();
		assert!(wants().await.is_empty(), "with no subject, nothing waits on a take");
		run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!(wants().await, waiting_on, "and with one again, it does");

		hear(&pool, "take-2", "From home.", None).await;
		let done = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert!(wants().await.is_empty(), "given: it waits on nothing");
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

	/// An essay stamped `day` whose scenes show `pictures`.
	fn showing_essay(day: &str, pictures: &[Value]) -> Vec<u8> {
		let shots: Vec<Value> = pictures.iter().map(|picture| serde_json::json!({ "picture": picture })).collect();
		let mut essay: Value = serde_json::from_slice(&essay(day)).unwrap();
		essay["shots"] = Value::Array(shots);
		essay.to_string().into_bytes()
	}

	/// An essay held for a snip not fetched yet waits, and not on a take:
	/// only a take not heard yet is the transcriber's waiter
	/// (`crate::stage::Waiter::CardsEssays`).
	#[tokio::test]
	async fn an_essay_waiting_on_a_snip_is_not_waiting_on_a_take() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let mut mine = given(dir.path());
		mine.subject = Some(String::from(ME));
		mine.snips = Some(dir.path().join("snips"));
		let fake = Fake::default();
		fake.ok("index.json", &index(&["snipped-2026-11-21"]));
		let snip = serde_json::json!({ "kind": "screen", "of": { "from": "snips", "id": "web:/settings:phone:1.2.3:0123456789ab", "commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "at": [] } });
		fake.ok("snipped-2026-11-21.json", &showing_essay("2026-11-21", &[snip]));

		let done = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!((done.exported, done.refused, done.waiting, done.waiting_on_takes), (0, 0, 1, 0));
		let wanted: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stage_want").fetch_one(&pool).await.unwrap();
		assert_eq!(wanted, 0, "a snip is not the transcriber's to hear");
	}

	/// A clip frame a scene shows is given beside the essay in `<key>.clips/`
	/// once home holds it, and the essay is taken back once the frame is gone.
	/// A snip refuses its essay when home fetches none (no token). No subject:
	/// neither is a person's.
	#[tokio::test]
	async fn a_scene_is_given_the_clip_frames_it_shows_and_refused_a_snip() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let mine = given(dir.path());
		let fake = Fake::default();
		fake.ok("index.json", &index(&["framed-2026-11-20", "snipped-2026-11-21"]));
		let frame = serde_json::json!({ "kind": "screen", "of": { "from": "clips", "clip": "studio-promo", "side": "after", "file": "01.png" } });
		let snip = serde_json::json!({ "kind": "screen", "of": { "from": "snips", "id": "web:/settings:phone:1.2.3:0123456789ab", "commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "at": [] } });
		fake.ok("framed-2026-11-20.json", &showing_essay("2026-11-20", &[frame]));
		fake.ok("snipped-2026-11-21.json", &showing_essay("2026-11-21", &[snip]));
		let status = dir.path().join("status");

		let done = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!((done.exported, done.refused), (0, 2));
		assert_eq!(
			std::fs::read_to_string(status.join("framed-2026-11-20.refused")).unwrap(),
			"reason=clip-missing\nitems=studio-promo/after/01.png\ncount=1\n"
		);
		assert_eq!(
			std::fs::read_to_string(status.join("snipped-2026-11-21.refused")).unwrap(),
			"reason=snip-missing\nitems=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa web:/settings:phone:1.2.3:0123456789ab\ncount=1\n"
		);
		let log = std::fs::read_to_string(status.join("framed-2026-11-20.failed.log")).unwrap();
		assert!(log.ends_with(": studio-promo/after/01.png\n"), "{log}");

		let mut conn = pool.acquire().await.unwrap();
		let kept = clip_frame_repo::Frame {
			clip_id: "studio-promo",
			side: "after",
			file: "01.png",
			made: clip_frame_repo::Made::Walk,
			bytes: b"frame-png",
			taken_at: "2026-11-21T09:00:00.000Z",
		};
		clip_frame_repo::keep(&mut conn, &kept).await.unwrap();
		drop(conn);
		let frame_at = mine.frames.join("studio-promo").join("after").join("01.png");
		std::fs::create_dir_all(frame_at.parent().unwrap()).unwrap();
		std::fs::write(&frame_at, b"other").unwrap();
		run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert!(
			!dir.path().join("framed-2026-11-20.json").exists(),
			"a frame on the drive that is not its row's is no frame"
		);
		std::fs::write(&frame_at, b"frame-png").unwrap();
		let done = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!((done.exported, done.refused), (1, 1));
		assert!(!status.join("framed-2026-11-20.refused").exists(), "its refusal goes");
		let clips = dir.path().join("framed-2026-11-20.clips");
		assert_eq!(names_in(&clips), ["studio-promo"]);
		assert_eq!(
			std::fs::read(clips.join("studio-promo/after/01.png")).unwrap(),
			b"frame-png",
			"the frame alone, under its path"
		);

		assert_eq!(tidy(&pool, &mine, NOW).await.unwrap(), Tidied::default(), "still held");
		std::fs::remove_file(&frame_at).unwrap();
		let tidied = tidy(&pool, &mine, NOW).await.unwrap();
		assert_eq!((tidied.taken_back, tidied.failed), (1, 0));
		assert!(!dir.path().join("framed-2026-11-20.json").exists() && !clips.exists(), "the essay and its folder go");
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
		assert_eq!(
			std::fs::read_to_string(status.join("gone-2026-11-19.refused")).unwrap(),
			"reason=take-missing\nitems=nope\ncount=1\n"
		);
		assert_eq!(
			std::fs::read_to_string(status.join("mumbled-2026-11-20.refused")).unwrap(),
			"reason=take-unheard\nitems=take-3\ncount=1\n"
		);
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
	/// no longer holds (its audio gone, no receipt) is taken back from the
	/// worker, and refused next pass.
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
		assert_eq!(tidy(&pool, &mine, NOW).await.unwrap(), Tidied::default(), "nothing to tidy yet");

		std::fs::create_dir_all(mine.renders.join("made-2026-11-19")).unwrap();
		std::fs::write(mine.renders.join("made-2026-11-19").join(crate::cards_essays::VIDEO), b"mp4").unwrap();
		sqlx::query("DELETE FROM voice_note_audio WHERE note_id = 'take-2'").execute(&pool).await.unwrap();
		assert_eq!(
			tidy(&pool, &mine, NOW).await.unwrap(),
			Tidied {
				takes_removed: 1,
				taken_back: 1,
				..Tidied::default()
			}
		);
		assert_eq!(names_in(dir.path()), ["made-2026-11-19.json", "renders"], "the made one keeps its essay alone");
		let next = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!((next.exported, next.refused), (0, 1), "the taken-back one is refused, not given again");
		assert_eq!(
			std::fs::read_to_string(dir.path().join("status").join("kept-2026-11-20.refused")).unwrap(),
			"reason=take-missing\nitems=take-2\ncount=1\n"
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
		assert_eq!((done.exported, done.refused, done.waiting, done.withdrawn), (0, 4, 0, 1), "a deleted take withdraws, #1055");
		let reason = |key: &str| std::fs::read_to_string(dir.path().join("status").join(String::from(key) + ".refused")).unwrap();
		for key in &keys[..3] {
			assert!(reason(key).starts_with("reason=take-unheard\nitems=t-") && reason(key).ends_with("\ncount=1\n"), "{key}");
		}
		assert!(!dir.path().join("status").join("deleted-2026-11-19.refused").exists(), "withdrawn, not refused");
		assert_eq!(reason("late-2026-11-20"), "reason=take-missing\nitems=t-late\ncount=1\n", "not sent yet: missing");

		hold_take(&pool, "t-late").await;
		let later = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!((later.refused, later.waiting, later.withdrawn), (3, 1, 0));
		let status = dir.path().join("status");
		assert!(
			!status.join("late-2026-11-20.refused").exists() && !status.join("late-2026-11-20.failed.log").exists(),
			"home now, waiting to be heard: no longer refused"
		);

		assert_eq!(tidy(&pool, &mine, NOW).await.unwrap(), Tidied::default());
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

	/// Deletes `take` as My data does (`DELETE /voice-notes/:id`): the note
	/// goes, its receipt stays.
	async fn delete_from_my_data(pool: &SqlitePool, take: &str) {
		let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
		let deleted = voice_repo::delete::delete(&mut tx, ME, take, voice_repo::delete::SWITCHED_ON, NOW).await.unwrap();
		tx.commit().await.unwrap();
		assert!(matches!(deleted, voice_repo::delete::Deleted::Gone(_)), "{take}: {deleted:?}");
	}

	/// Where a render of `key` stands when its take is deleted.
	#[derive(Debug, Clone, Copy)]
	enum Stage {
		/// Given, not started: its `<key>.json` and takes wait.
		Given,
		/// Under way: the worker has work and a half-made folder.
		Rendering,
		/// Done: its video is made, its takes gone.
		Made,
	}

	/// The worker's files of `key` at `stage`, under `given`.
	fn stand(given: &Given, key: &str, stage: Stage) {
		match stage {
			Stage::Given => {}
			Stage::Rendering => {
				std::fs::create_dir_all(given.work.join(key)).unwrap();
				std::fs::write(given.work.join(key).join("take-1.wav"), b"RIFF").unwrap();
				std::fs::create_dir_all(given.renders.join(key)).unwrap();
				std::fs::write(given.renders.join(key).join("shot-times.json"), b"{}").unwrap();
			}
			Stage::Made => {
				std::fs::create_dir_all(given.renders.join(key)).unwrap();
				std::fs::write(given.renders.join(key).join(crate::cards_essays::VIDEO), b"mp4 with my voice").unwrap();
			}
		}
	}

	/// paulgsc/server#1055: a take deleted from My data withdraws its essay
	/// for good before, during or after its render, as a Rust Rounds week is
	/// withdrawn: both receipts, its video and folder gone, never given
	/// again, no longer listed, and the take's own receipt kept.
	#[tokio::test]
	async fn a_take_deleted_before_during_or_after_the_render_withdraws_its_essay_for_good() {
		for stage in [Stage::Given, Stage::Rendering, Stage::Made] {
			withdrawn_at(stage).await;
		}
	}

	async fn withdrawn_at(stage: Stage) {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let fake = Fake::default();
		let (key, other) = ("spoken-2026-11-19", "kept-2026-11-20");
		fake.ok("index.json", &index(&[key, other]));
		fake.ok(&(String::from(key) + ".json"), &spoken_essay("2026-11-19", &["take-1", "take-2"]));
		fake.ok(&(String::from(other) + ".json"), &spoken_essay("2026-11-20", &["take-3"]));
		for take in ["take-1", "take-2", "take-3"] {
			hold_take(&pool, take).await;
			hear(&pool, take, "Hello there.", Some("heard")).await;
		}
		let mut mine = given(&dir.path().join("essays"));
		mine.subject = Some(String::from(ME));
		assert_eq!(run_once(&pool, &fake, &mine, &clock).await.unwrap().exported, 2, "{stage:?}");
		stand(&mine, key, stage);
		if matches!(stage, Stage::Made) {
			assert_eq!(tidy(&pool, &mine, NOW).await.unwrap().takes_removed, 1, "made: its takes go first");
			let mut conn = pool.acquire().await.unwrap();
			cards_essay_repo::record_link(&mut conn, key, "dQw4w9WgXcQ", NOW).await.unwrap();
		}
		assert_eq!(tidy(&pool, &mine, NOW).await.unwrap(), Tidied::default(), "{stage:?}: nothing deleted, nothing to do");

		delete_from_my_data(&pool, "take-2").await;
		assert_eq!(
			tidy(&pool, &mine, NOW).await.unwrap(),
			Tidied {
				withdrawn: 1,
				..Tidied::default()
			},
			"{stage:?}"
		);
		let essays = &mine.essays;
		assert!(
			!essays.join(String::from(key) + ".json").exists()
				&& !essays.join(String::from(key) + ".takes").exists()
				&& !mine.renders.join(key).exists()
				&& !mine.work.join(key).exists(),
			"{stage:?}: every file of it goes"
		);
		assert!(essays.join(String::from(other) + ".json").exists(), "{stage:?}: another essay is not touched");
		let mut conn = pool.acquire().await.unwrap();
		let withdrawn = cards_essay_repo::withdrawn(&mut conn, ME).await.unwrap();
		assert_eq!(
			withdrawn.iter().map(|w| (w.key.as_str(), w.note.as_str(), w.youtube_id.as_deref())).collect::<Vec<_>>(),
			[(key, "take-2", matches!(stage, Stage::Made).then_some("dQw4w9WgXcQ"))],
			"{stage:?}: the receipt names the take, and the link it had"
		);
		let size = withdrawn[0].size;
		assert_eq!(size > 0, !matches!(stage, Stage::Given), "{stage:?}: the bytes that went");
		assert!(
			voice_repo::delete::receipt(&mut conn, ME, "take-2").await.unwrap().is_some(),
			"the take's own receipt stays"
		);
		drop(conn);

		// A render under way may still finish after its essay was withdrawn:
		// the next minute removes what it made.
		std::fs::create_dir_all(mine.renders.join(key)).unwrap();
		std::fs::write(mine.renders.join(key).join(crate::cards_essays::VIDEO), b"late").unwrap();
		assert_eq!(tidy(&pool, &mine, NOW).await.unwrap(), Tidied { removed: 1, ..Tidied::default() }, "{stage:?}");
		assert!(!mine.renders.join(key).exists(), "{stage:?}: a late video goes within a minute");
		assert_eq!(tidy(&pool, &mine, NOW).await.unwrap(), Tidied::default(), "{stage:?}: then nothing is left");

		let next = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!((next.exported, next.refused, next.waiting), (0, 0, 0), "{stage:?}: never given again, nor said refused");
		assert!(!essays.join(String::from(key) + ".json").exists());
		assert!(!mine.status.join(String::from(key) + ".refused").exists());
		let held: Vec<_> = crate::cards_essays::held(&pool).await.unwrap().into_iter().map(|held| held.key).collect();
		assert_eq!(held, [other], "{stage:?}: off Studio › Renders");
		let mut conn = pool.acquire().await.unwrap();
		let feed: Vec<_> = crate::feeds::Feed::CardsEssay.held(&mut conn, ME).await.unwrap().into_iter().map(|held| held.key).collect();
		assert_eq!(feed, [other], "{stage:?}: off the episodes");
		assert!(
			!crate::feeds::Feed::CardsEssay.holds(&mut conn, ME, key).await.unwrap(),
			"{stage:?}: its video is no one's to fetch"
		);
	}

	/// An essay never given, waiting for a take to be heard, is withdrawn by
	/// the pass once another take it plays is deleted: no refusal, and off
	/// every list, with nothing on the drive to remove.
	#[tokio::test]
	async fn an_essay_never_given_is_withdrawn_by_the_pass_not_refused() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let fake = Fake::default();
		let key = "waiting-2026-11-19";
		fake.ok("index.json", &index(&[key]));
		fake.ok(&(String::from(key) + ".json"), &spoken_essay("2026-11-19", &["take-1", "take-2"]));
		for take in ["take-1", "take-2"] {
			hold_take(&pool, take).await;
		}
		hear(&pool, "take-1", "Hello there.", Some("heard")).await;
		let mut mine = given(&dir.path().join("essays"));
		mine.subject = Some(String::from(ME));
		let first = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!((first.exported, first.waiting, first.withdrawn), (0, 1, 0), "take-2 not heard yet");

		delete_from_my_data(&pool, "take-1").await;
		let next = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!((next.exported, next.refused, next.waiting, next.withdrawn), (0, 0, 0, 1));
		assert!(!mine.status.join(String::from(key) + ".refused").exists(), "withdrawn, not said refused");
		let mut conn = pool.acquire().await.unwrap();
		let withdrawn = cards_essay_repo::withdrawn(&mut conn, ME).await.unwrap();
		assert_eq!(
			withdrawn.iter().map(|w| (w.key.as_str(), w.note.as_str(), w.size)).collect::<Vec<_>>(),
			[(key, "take-1", 0)]
		);
		drop(conn);
		assert!(crate::cards_essays::held(&pool).await.unwrap().is_empty(), "off Studio › Renders");
		let again = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!((again.exported, again.refused, again.withdrawn), (0, 0, 0), "never looked at again");
		assert_eq!(tidy(&pool, &mine, NOW).await.unwrap(), Tidied::default());
	}

	/// A deleted take withdraws an essay never given though another take it
	/// plays does not read back; and a withdrawn essay's status files and a
	/// stopped export's partial go with the rest of it.
	#[tokio::test]
	async fn a_deleted_take_withdraws_past_an_unreadable_one_and_nothing_of_it_is_left() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let fake = Fake::default();
		let key = "mixed-2026-11-19";
		fake.ok("index.json", &index(&[key]));
		fake.ok(&(String::from(key) + ".json"), &spoken_essay("2026-11-19", &["take-2", "take-1"]));
		for take in ["take-1", "take-2"] {
			hold_take(&pool, take).await;
		}
		hear(&pool, "take-1", "Hello there.", Some("heard")).await;
		// Heard, and its words gone: it does not read back. The table's check
		// refuses such a row, so it is written past it, as a damaged drive
		// might leave one.
		sqlx::query("PRAGMA ignore_check_constraints = ON").execute(&pool).await.unwrap();
		sqlx::query(
			"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, language, language_p, duration_ms, text, took_ms, heard_at)
			 VALUES (?1, 'take-2', 'm', 'e', 1, 'heard', 'en', 1.0, 900, NULL, 1, '2026-11-20T10:01:00.000Z')",
		)
		.bind(ME)
		.execute(&pool)
		.await
		.unwrap();
		sqlx::query("PRAGMA ignore_check_constraints = OFF").execute(&pool).await.unwrap();
		let mut mine = given(&dir.path().join("essays"));
		mine.subject = Some(String::from(ME));
		assert_eq!(run_once(&pool, &fake, &mine, &clock).await.unwrap().failures, ["unreadable"], "take-2 does not read back");

		delete_from_my_data(&pool, "take-1").await;
		let next = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!((next.withdrawn, next.failures.len()), (1, 0), "withdrawn, not held by take-2");

		// What a worker said of it after its folders went, and a partial a
		// stopped export left.
		std::fs::create_dir_all(&mine.status).unwrap();
		std::fs::write(mine.status.join(String::from(key) + ".failed.log"), b"gone").unwrap();
		std::fs::write(mine.essays.join(String::from(".") + key + ".json.partial"), b"{").unwrap();
		std::fs::create_dir_all(mine.work.join(key)).unwrap();
		assert_eq!(tidy(&pool, &mine, NOW).await.unwrap().removed, 1);
		assert!(!mine.status.join(String::from(key) + ".failed.log").exists(), "its status goes");
		assert!(!mine.essays.join(String::from(".") + key + ".json.partial").exists(), "its partial goes");
		assert!(!mine.work.join(key).exists());
	}

	/// An essay with no spoken card, or whose takes are all held, is never
	/// withdrawn, its video made or not; and with no subject, a made video is
	/// left as it is.
	#[tokio::test]
	async fn only_a_deleted_take_an_essay_plays_withdraws_it() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let fake = Fake::default();
		fake.ok("index.json", &index(&["silent-2026-11-18", "spoken-2026-11-19"]));
		fake.ok("silent-2026-11-18.json", &essay("2026-11-18"));
		fake.ok("spoken-2026-11-19.json", &spoken_essay("2026-11-19", &["take-1"]));
		for take in ["take-1", "take-9"] {
			hold_take(&pool, take).await;
			hear(&pool, take, "Hello there.", Some("heard")).await;
		}
		let mut mine = given(&dir.path().join("essays"));
		mine.subject = Some(String::from(ME));
		assert_eq!(run_once(&pool, &fake, &mine, &clock).await.unwrap().exported, 2);
		for key in ["silent-2026-11-18", "spoken-2026-11-19"] {
			stand(&mine, key, Stage::Made);
		}
		assert_eq!(tidy(&pool, &mine, NOW).await.unwrap().takes_removed, 1);
		delete_from_my_data(&pool, "take-9").await;
		assert_eq!(tidy(&pool, &mine, NOW).await.unwrap(), Tidied::default(), "a take no essay plays");
		mine.subject = None;
		assert_eq!(tidy(&pool, &mine, NOW).await.unwrap(), Tidied::default(), "no subject: nothing given with takes is left");
		assert!(mine.renders.join("spoken-2026-11-19").join(crate::cards_essays::VIDEO).exists());
		let mut conn = pool.acquire().await.unwrap();
		assert!(cards_essay_repo::gone(&mut conn).await.unwrap().is_empty());
	}

	/// paulgsc/some-ui#3339, the Rust Rounds case: an essay refused
	/// (`take-unheard`) for two takes every attempt failed on stays refused
	/// while one of them is not asked again; once the owner asks home to hear
	/// both again, it waits instead, its takes are wanted (`stage_want`, so
	/// the transcriber hears them first, #1098), and a fast-track on one is
	/// taken and puts it at the head of the line (#1100).
	#[tokio::test]
	async fn an_essay_refused_for_takes_home_hears_again_waits_and_wants_them_first() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let fake = Fake::default();
		let key = "rust-rounds-how-it-works-2026-11-20";
		let takes = ["37c56646-e792-4323-896b-453592377aef", "4499b000-e757-48f8-87d7-50eeda07029e"];
		fake.ok("index.json", &index(&[key]));
		fake.ok(&(String::from(key) + ".json"), &spoken_essay("2026-11-20", &takes));
		let (plain, verbatim) = crate::transcribe::keys();
		let keys = voice_repo::transcript::Keys {
			plain: &plain,
			verbatim: &verbatim,
		};
		for take in takes {
			hold_take(&pool, take).await;
			for n in 1..=3 {
				sqlx::query(
					"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, took_ms, heard_at)
					 VALUES (?1, ?2, ?3, 'e', ?4, 'deadline', 1, 'then')",
				)
				.bind(ME)
				.bind(take)
				.bind(&plain)
				.bind(n)
				.execute(&pool)
				.await
				.unwrap();
			}
			sqlx::query(
				"INSERT INTO voice_transcript_verdict (subject_id, note_id, model, attempt, rule, verdict, reasons, judged_at)
				 VALUES (?1, ?2, ?3, 3, 'heard-1', 'failed', '[\"deadline\"]', 'now')",
			)
			.bind(ME)
			.bind(take)
			.bind(&plain)
			.execute(&pool)
			.await
			.unwrap();
		}
		let mut mine = given(dir.path());
		mine.subject = Some(String::from(ME));
		let wants = || async {
			let mut rows: Vec<String> = sqlx::query_scalar("SELECT item_key FROM stage_want WHERE stage = 'transcriber'")
				.fetch_all(&pool)
				.await
				.unwrap();
			rows.sort();
			rows
		};
		let done = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!((done.refused, done.waiting), (1, 0), "every attempt failed: refused");
		assert!(wants().await.is_empty(), "a refused essay wants nothing");

		let ask = |take: &'static str| {
			let pool = pool.clone();
			async move {
				let mut conn = pool.acquire().await.unwrap();
				voice_repo::attention::hear_again(&mut conn, ME, take, Some(keys), "2026-11-22T09:00:00.000Z")
					.await
					.unwrap()
			}
		};
		assert!(matches!(ask(takes[0]).await, voice_repo::attention::Acted::Recorded(_)));
		let one = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!((one.refused, one.waiting), (1, 0), "the other take is still never to be heard");
		assert!(matches!(ask(takes[1]).await, voice_repo::attention::Acted::Recorded(_)));
		let later = run_once(&pool, &fake, &mine, &clock).await.unwrap();
		assert_eq!((later.refused, later.waiting, later.waiting_on_takes), (0, 1, 1), "both heard again: it waits on them");
		assert!(!dir.path().join("status").join(String::from(key) + ".refused").exists(), "no longer said refused");
		assert_eq!(wants().await, takes, "its takes are wanted, so heard first");

		let mut conn = pool.acquire().await.unwrap();
		let fast = voice_repo::fast_track::act(&mut conn, ME, takes[1], voice_repo::fast_track::Act::Fast, keys, crate::transcribe::pass::ATTEMPTS, "now")
			.await
			.unwrap();
		assert_eq!(fast, voice_repo::fast_track::Acted::Recorded, "a take heard again may be put first");
		let order: Vec<_> = voice_repo::transcript::waiting(&mut conn, keys, crate::transcribe::pass::ATTEMPTS, 10)
			.await
			.unwrap()
			.into_iter()
			.map(|waiting| (waiting.note_id, waiting.urgency))
			.collect();
		assert_eq!(
			order,
			[
				(String::from(takes[1]), voice_repo::transcript::Urgency::FastTrack),
				(String::from(takes[0]), voice_repo::transcript::Urgency::Wanted),
			],
			"under their fresh keys: the fast-tracked take first, then the wanted one"
		);
	}
}
