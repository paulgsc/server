//! A template's preview (SFEED12, paulgsc/some-ui#2757).
//!
//! A short video of a show's template drawn from its sample, which the owner
//! asks home for from Studio, to see a template change before any episode
//! carries it.
//!
//! It keeps no table: everything is a file under `STORAGE`, as the call's
//! renders are.
//!
//! - **The request.** `POST /previews/:template` writes `<id>.json` to
//!   [`REQUESTS_FOLDER`], `<id>` being `<template>-<UTC yyyymmddThhmmssZ>`.
//!   The render worker (`infra/render/render-calls.sh`, its `preview` job)
//!   reads that folder, as it reads the call's weeks, and renders the
//!   preview under the lane (L4c), one job like any other: the lane decides
//!   when it runs, and it waits, renders, pauses and gives way as a week
//!   does.
//! - **The preview.** The worker keeps it whole in
//!   `renders/preview/<id>/` ([`RENDERS_FOLDER`]): `preview.mp4` ([`VIDEO`])
//!   and the sample it recorded, which names the template's pinned version
//!   (the call's week, `week.json`; Playoff Delta's fill, `narration.json`). A failed attempt's output is
//!   `renders/preview-worker/<id>.failed.log`. A failed preview is never
//!   tried again: the owner asks for another.
//! - **Its state** is read from those files and the lane's `job.state`
//!   ([`read`]), never kept here. Studio › Renders lists each preview from
//!   the same files (`feeds::renders`, its `preview` shelf).
//! - **One at a time per template.** A request while one of the template is
//!   still to be made answers that one ([`request`]).
//! - **The deadline.** A request not ready [`LIFETIME`] after it was asked
//!   (by its id's stamp) ends as failed, `overdue`: [`prune`] replaces its
//!   `<id>.json` with `<id>.overdue`, so the worker lets it go if it waits, a
//!   render that died never sits at `rendering` for good, and the owner can
//!   ask again at once.
//! - **The prune** ([`prune`], every [`PRUNE_EVERY`]): a preview goes
//!   [`LIFETIME`] after it is made, and an older preview of its template as
//!   soon as a newer one is ready; a failed request goes [`LIFETIME`] after
//!   it failed, its work at once. Each goes with its request, its status and
//!   its work, and anything of a preview whose request is gone (a render
//!   that landed after it, a removal cut short) goes too. The owner asked for
//!   exactly this, with no switch and no receipt (paulgsc/some-ui#2757): a
//!   preview is a throwaway made from sample data, never the owner's. The
//!   job in progress is never pruned. So home keeps, per template, at most
//!   one request still to be made, one ready preview, and the last day's
//!   failures.
//!
//! Nothing here names a person or reads a subject: a preview is of a
//! template's sample, and the routes are a paired phone's (`routes::previews`).

use crate::AppState;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};

/// Where `file_host` writes each request: `${STORAGE}/preview-requests`, read
/// only by the render worker, at `/previews`.
pub const REQUESTS_FOLDER: &str = "/app/dbs/preview-requests";

/// Where the render worker keeps each finished preview, one folder per id.
pub const RENDERS_FOLDER: &str = "/app/dbs/renders/preview";

/// Where the render worker keeps a preview's failed attempt and its log.
pub const STATUS_FOLDER: &str = "/app/dbs/renders/preview-worker";

/// Where the render worker keeps a preview's work while it is made.
pub const WORK_FOLDER: &str = "/app/dbs/renders/preview-work";

/// The video in a preview's folder.
pub const VIDEO: &str = "preview.mp4";

/// The templates a preview may be of: paulgsc/some-ui
/// `packages/video-render/src/templates.ts`, `TEMPLATE_IDS`, and the render
/// worker's `PREVIEW_ID`. Any other is `404`.
pub const TEMPLATES: [&str; 4] = ["conference-call", "playoff-delta-legend", "playoff-delta-week", "rust-essay"];

/// How long a preview lives after it is made, and a failed one after it
/// failed.
pub const LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

/// How often [`prune`] runs.
pub const PRUNE_EVERY: Duration = Duration::from_secs(10 * 60);

/// What a request not made within [`LIFETIME`] of being asked is renamed
/// to: failed, `overdue`.
const OVERDUE: &str = ".overdue";

/// How long a half-written request (`.<id>.json`, a write cut short) is left
/// before the prune removes it.
const PARTIAL_AFTER: Duration = Duration::from_secs(60);

/// The most bytes of a failed attempt's log read for its reason.
const FAILED_LOG_CEILING: u64 = 64 * 1024;

/// The most bytes of a preview's `week.json` or `narration.json` read for its
/// version.
const SAMPLE_CEILING: u64 = 1024 * 1024;

/// Held across a request and a prune, so a prune never removes a request
/// being written, and two requests never make two previews of a template.
static FOLDERS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The folders a preview lives in: [`Folders::home`] on home, a temporary
/// folder in tests.
#[derive(Debug, Clone)]
pub struct Folders {
	pub requests: PathBuf,
	pub renders: PathBuf,
	pub status: PathBuf,
	pub work: PathBuf,
	/// The lane's folder, where the render worker writes `job.state`.
	pub lane: PathBuf,
}

impl Folders {
	/// Home's.
	#[must_use]
	pub fn home() -> Self {
		Self {
			requests: PathBuf::from(REQUESTS_FOLDER),
			renders: PathBuf::from(RENDERS_FOLDER),
			status: PathBuf::from(STATUS_FOLDER),
			work: PathBuf::from(WORK_FOLDER),
			lane: PathBuf::from(crate::call_weeks::render::STATUS_FOLDER),
		}
	}
}

/// Where a preview is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
	/// Asked for, waiting for its turn or for room on home.
	Waiting,
	Rendering,
	/// Frozen while home's interactive work runs.
	Paused,
	/// Stopped by the lane for room; it resumes from its kept steps.
	GivenWay,
	Ready,
	Failed,
}

impl State {
	/// Whether it is still to be made.
	#[must_use]
	pub const fn pending(self) -> bool {
		matches!(self, Self::Waiting | Self::Rendering | Self::Paused | Self::GivenWay)
	}
}

/// A preview as the phone reads it (paulgsc/some-ui#2778).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Preview {
	pub id: String,
	/// The template's pinned version it was made from, once it is made.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub version: Option<u32>,
	pub state: State,
	/// Unix seconds, when ready.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub made_at: Option<u64>,
	/// Unix seconds, when ready: when the prune takes it.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub expires_at: Option<u64>,
	/// The video's bytes, when ready.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub bytes: Option<u64>,
	/// Why it failed, as a short class, never its log.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub failed: Option<&'static str>,
}

/// Whether `template` is one a preview may be of.
#[must_use]
pub fn is_template(template: &str) -> bool {
	TEMPLATES.contains(&template)
}

/// `id`'s template, if `id` is `<template>-<yyyymmddThhmmssZ>` of a known one.
#[must_use]
pub fn template_of(id: &str) -> Option<&'static str> {
	let (template, stamp) = id.rsplit_once('-')?;
	let stamp = stamp.as_bytes();
	let stamped = stamp.len() == 16 && stamp[..8].iter().all(u8::is_ascii_digit) && stamp[8] == b'T' && stamp[9..15].iter().all(u8::is_ascii_digit) && stamp[15] == b'Z';
	if !stamped {
		return None;
	}
	TEMPLATES.iter().copied().find(|known| *known == template)
}

/// The id of a preview of `template` asked for at `now`.
#[must_use]
pub fn id_at(template: &str, now: SystemTime) -> String {
	let at: chrono::DateTime<chrono::Utc> = now.into();
	String::from(template) + "-" + &at.format("%Y%m%dT%H%M%SZ").to_string()
}

/// When preview `id` was asked for, by its stamp, in Unix seconds.
#[must_use]
pub fn asked_at(id: &str) -> Option<u64> {
	template_of(id)?;
	let (_, stamp) = id.rsplit_once('-')?;
	let at = chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%dT%H%M%SZ").ok()?;
	u64::try_from(at.and_utc().timestamp()).ok()
}

fn unix(at: SystemTime) -> u64 {
	at.duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs())
}

/// Every request of `template`, oldest first: the stamp sorts as its time.
async fn ids_of(folders: &Folders, template: &str) -> std::io::Result<Vec<String>> {
	let mut ids = Vec::new();
	let mut entries = match tokio::fs::read_dir(&folders.requests).await {
		Ok(entries) => entries,
		Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(ids),
		Err(err) => return Err(err),
	};
	while let Some(entry) = entries.next_entry().await? {
		let name = entry.file_name();
		let Some(id) = name.to_str().and_then(|name| name.strip_suffix(".json").or_else(|| name.strip_suffix(OVERDUE))) else {
			continue;
		};
		if template_of(id) == Some(template) && id.len() == template.len() + 17 {
			ids.push(id.to_owned());
		}
	}
	ids.sort_unstable();
	ids.dedup();
	Ok(ids)
}

/// Reads at most `ceiling` bytes of `path`, if it is a plain file.
async fn read_small(path: &Path, ceiling: u64) -> Option<Vec<u8>> {
	use tokio::io::AsyncReadExt as _;
	let meta = tokio::fs::symlink_metadata(path).await.ok()?;
	if !meta.file_type().is_file() {
		return None;
	}
	let file = tokio::fs::File::open(path).await.ok()?;
	let mut bytes = Vec::new();
	file.take(ceiling).read_to_end(&mut bytes).await.ok()?;
	Some(bytes)
}

/// The job the render worker says is in progress: its key and state, if it
/// is a preview's.
async fn job_in_progress(folders: &Folders) -> Option<(String, String, u64)> {
	let text = read_small(&folders.lane.join("job.state"), 4096).await?;
	let text = String::from_utf8(text).ok()?;
	let (mut kind, mut key, mut state, mut yields) = (None, None, None, 0);
	for line in text.lines() {
		match line.split_once('=') {
			Some(("kind", value)) => kind = Some(value),
			Some(("key", value)) => key = Some(value),
			Some(("state", value)) => state = Some(value),
			Some(("yields", value)) => yields = value.parse().unwrap_or(0),
			_ => {}
		}
	}
	if kind? != "preview" {
		return None;
	}
	Some((key?.to_owned(), state?.to_owned(), yields))
}

/// Why a failed attempt failed, as a class: its log is the recorder's, and
/// never leaves home.
pub(crate) fn failure_class(log: &str) -> &'static str {
	// paulgsc/some-ui `tests/record/preview.ts` and
	// `packages/video-render/src/policy.ts`, which word each refusal.
	if log.contains("has no sample fill") {
		"no-sample"
	} else if log.contains("the video would run ") && log.contains("past the policy's") {
		"too-long"
	} else if log.contains("the video is ") && log.contains(" MB, past the policy's") {
		"too-large"
	} else {
		"render"
	}
}

/// The template's pinned version a finished preview was made from, as its
/// sample names it: the call's sample week (`week.json`, its `template`), or
/// Playoff Delta's fill as recorded (`narration.json`, its `format`), each
/// only when it names `template`.
async fn version_of(folder: &Path, template: &str) -> Option<u32> {
	for (file, key) in [("week.json", "template"), ("narration.json", "format")] {
		if let Some(version) = named_version(&folder.join(file), key, template).await {
			return Some(version);
		}
	}
	None
}

/// The `version` of `path`'s `key` object, when its `id` is `template`.
async fn named_version(path: &Path, key: &str, template: &str) -> Option<u32> {
	let bytes = read_small(path, SAMPLE_CEILING).await?;
	let sample: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
	let named = sample.get(key)?;
	if named.get("id").and_then(serde_json::Value::as_str) != Some(template) {
		return None;
	}
	named
		.get("version")?
		.as_u64()
		.and_then(|version| u32::try_from(version).ok())
		.filter(|version| *version > 0)
}

/// Where preview `id` is, from the worker's files.
///
/// # Errors
/// The drive refusing a read of a file that is there.
pub async fn read(folders: &Folders, id: &str) -> std::io::Result<Preview> {
	let template = template_of(id).unwrap_or_default();
	let mut preview = Preview {
		id: id.to_owned(),
		version: None,
		state: State::Waiting,
		made_at: None,
		expires_at: None,
		bytes: None,
		failed: None,
	};
	if tokio::fs::try_exists(folders.requests.join(String::from(id) + OVERDUE)).await? {
		preview.state = State::Failed;
		preview.failed = Some("overdue");
		return Ok(preview);
	}
	let folder = folders.renders.join(id);
	match tokio::fs::symlink_metadata(folder.join(VIDEO)).await {
		Ok(meta) if meta.file_type().is_file() => {
			let made = unix(meta.modified()?);
			preview.state = State::Ready;
			preview.made_at = Some(made);
			preview.expires_at = Some(made + LIFETIME.as_secs());
			preview.bytes = Some(meta.len());
			preview.version = version_of(&folder, template).await;
			return Ok(preview);
		}
		Ok(_) => {}
		Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
		Err(err) => return Err(err),
	}
	if let Some(log) = read_small(&folders.status.join(String::from(id) + ".failed.log"), FAILED_LOG_CEILING).await {
		preview.state = State::Failed;
		preview.failed = Some(failure_class(&String::from_utf8_lossy(&log)));
		return Ok(preview);
	}
	if let Some((key, state, yields)) = job_in_progress(folders).await {
		if key == id {
			preview.state = match state.as_str() {
				// `done`: rendered, being moved into place.
				"rendering" | "done" => State::Rendering,
				"paused" => State::Paused,
				_ if yields > 0 => State::GivenWay,
				_ => State::Waiting,
			};
		}
	}
	Ok(preview)
}

/// `template`'s preview now: its newest, unless that one has expired.
///
/// # Errors
/// The drive refusing a read.
pub async fn current(folders: &Folders, template: &str, now: SystemTime) -> std::io::Result<Option<Preview>> {
	let Some(newest) = ids_of(folders, template).await?.pop() else {
		return Ok(None);
	};
	let preview = read(folders, &newest).await?;
	let expired = preview.expires_at.is_some_and(|expires| expires <= unix(now));
	Ok((!expired).then_some(preview))
}

/// What [`request`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asked {
	/// A new preview is asked for.
	New(Preview),
	/// One of the template is still to be made: that one.
	Pending(Preview),
}

/// Asks for a preview of `template` at `now`, unless one is still to be
/// made.
///
/// # Errors
/// The drive refusing a read or the write.
pub async fn request(folders: &Folders, template: &str, now: SystemTime) -> std::io::Result<Asked> {
	let _held = FOLDERS.lock().await;
	if let Some(preview) = current(folders, template, now).await? {
		if preview.state.pending() {
			return Ok(Asked::Pending(preview));
		}
	}
	let id = id_at(template, now);
	let path = folders.requests.join(String::from(&id) + ".json");
	if tokio::fs::try_exists(&path).await? {
		// Asked again within the same second, after the last one failed or
		// was made: the same request.
		return Ok(Asked::Pending(read(folders, &id).await?));
	}
	let body = serde_json::json!({ "template": template, "asked_at": unix(now) }).to_string();
	// Written whole, so the worker never reads half a request.
	let partial = folders.requests.join(String::from(".") + &id + ".json");
	tokio::fs::write(&partial, body).await?;
	tokio::fs::rename(&partial, &path).await?;
	Ok(Asked::New(read(folders, &id).await?))
}

/// What a prune removed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Pruned {
	/// The ids removed, oldest first per template.
	pub removed: Vec<String>,
	/// The ids found overdue this pass.
	pub overdue: Vec<String>,
	/// What was left of a preview whose request was gone, by name.
	pub swept: Vec<String>,
}

/// Removes every preview past its life (module doc).
///
/// That is one [`LIFETIME`] after it was made, any older than its template's
/// newest ready one, and a failed one [`LIFETIME`] after it failed. Never the
/// job in progress.
///
/// # Errors
/// The drive refusing a read or a removal; what was removed before stays
/// removed.
pub async fn prune(folders: &Folders, now: SystemTime) -> std::io::Result<Pruned> {
	let _held = FOLDERS.lock().await;
	let mut pruned = Pruned::default();
	let running = job_in_progress(folders).await.map(|(key, ..)| key);
	for template in TEMPLATES {
		let ids = ids_of(folders, template).await?;
		let mut previews = Vec::with_capacity(ids.len());
		for id in &ids {
			previews.push(read(folders, id).await?);
		}
		let newest_ready = previews.iter().rposition(|preview| preview.state == State::Ready);
		for (at, preview) in previews.iter_mut().enumerate() {
			// The deadline holds even for the job in progress: a render that
			// died must not stay `rendering` for good. Its request is only
			// renamed, so nothing the worker writes is touched.
			if preview.state.pending() && asked_at(&preview.id).is_some_and(|asked| asked + LIFETIME.as_secs() <= unix(now)) {
				// Written fresh, not renamed: its time is when it was found
				// overdue, which the day before it goes counts from.
				let marker = serde_json::json!({ "overdue_at": unix(now) }).to_string();
				tokio::fs::write(folders.requests.join(String::from(&preview.id) + OVERDUE), marker).await?;
				remove_file(&folders.requests.join(String::from(&preview.id) + ".json")).await?;
				preview.state = State::Failed;
				preview.failed = Some("overdue");
				pruned.overdue.push(preview.id.clone());
			}
			if running.as_deref() == Some(preview.id.as_str()) {
				continue;
			}
			if preview.state == State::Failed {
				// A failed preview's work is never used again.
				remove_dir(&folders.work.join(&preview.id)).await?;
			}
			let superseded = newest_ready.is_some_and(|newest| at < newest);
			let expired = preview.expires_at.is_some_and(|expires| expires <= unix(now));
			let failed_long_ago = preview.state == State::Failed && failed_at(folders, &preview.id).await.is_some_and(|at| at + LIFETIME.as_secs() <= unix(now));
			if superseded || expired || failed_long_ago {
				remove(folders, &preview.id).await?;
				pruned.removed.push(preview.id.clone());
			}
		}
	}
	sweep(folders, running.as_deref(), now, &mut pruned).await?;
	Ok(pruned)
}

/// Removes what is left of a preview whose request is gone: a render that
/// landed after it, or a removal cut short. And a half-written request
/// [`PARTIAL_AFTER`] old. Never the job in progress, and never a name that is
/// no preview's.
async fn sweep(folders: &Folders, running: Option<&str>, now: SystemTime, pruned: &mut Pruned) -> std::io::Result<()> {
	let requested = |id: &str| {
		let (json, overdue) = (folders.requests.join(String::from(id) + ".json"), folders.requests.join(String::from(id) + OVERDUE));
		async move { Ok::<_, std::io::Error>(tokio::fs::try_exists(json).await? || tokio::fs::try_exists(overdue).await?) }
	};
	for folder in [&folders.renders, &folders.work] {
		for name in names(folder).await? {
			if template_of(&name).is_none() || running == Some(name.as_str()) || requested(&name).await? {
				continue;
			}
			remove_dir(&folder.join(&name)).await?;
			pruned.swept.push(name);
		}
	}
	for name in names(&folders.status).await? {
		let Some(id) = name.strip_suffix(".failed.log").or_else(|| name.strip_suffix(".cache.log")) else {
			continue;
		};
		if template_of(id).is_none() || running == Some(id) || requested(id).await? {
			continue;
		}
		remove_file(&folders.status.join(&name)).await?;
		pruned.swept.push(name);
	}
	for name in names(&folders.requests).await? {
		let Some(id) = name.strip_prefix('.').and_then(|name| name.strip_suffix(".json")) else {
			continue;
		};
		let path = folders.requests.join(&name);
		let old = tokio::fs::metadata(&path)
			.await
			.ok()
			.and_then(|meta| meta.modified().ok())
			.is_some_and(|at| at + PARTIAL_AFTER <= now);
		if template_of(id).is_some() && old {
			remove_file(&path).await?;
			pruned.swept.push(name);
		}
	}
	Ok(())
}

/// The names in `folder`, none when it is missing.
async fn names(folder: &Path) -> std::io::Result<Vec<String>> {
	let mut names = Vec::new();
	let mut entries = match tokio::fs::read_dir(folder).await {
		Ok(entries) => entries,
		Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(names),
		Err(err) => return Err(err),
	};
	while let Some(entry) = entries.next_entry().await? {
		if let Some(name) = entry.file_name().to_str() {
			names.push(name.to_owned());
		}
	}
	Ok(names)
}

/// When preview `id` failed: when it was found overdue, else its failed
/// attempt's log's time.
async fn failed_at(folders: &Folders, id: &str) -> Option<u64> {
	let overdue = tokio::fs::metadata(folders.requests.join(String::from(id) + OVERDUE)).await;
	let meta = match overdue {
		Ok(meta) => meta,
		Err(_) => tokio::fs::metadata(folders.status.join(String::from(id) + ".failed.log")).await.ok()?,
	};
	meta.modified().ok().map(unix)
}

/// Removes everything of preview `id`, its request first, so the worker never
/// takes it up again meanwhile.
async fn remove(folders: &Folders, id: &str) -> std::io::Result<()> {
	remove_file(&folders.requests.join(String::from(id) + ".json")).await?;
	remove_file(&folders.requests.join(String::from(id) + OVERDUE)).await?;
	remove_dir(&folders.renders.join(id)).await?;
	remove_dir(&folders.work.join(id)).await?;
	for suffix in [".failed.log", ".cache.log"] {
		remove_file(&folders.status.join(String::from(id) + suffix)).await?;
	}
	Ok(())
}

async fn remove_file(path: &Path) -> std::io::Result<()> {
	match tokio::fs::remove_file(path).await {
		Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
		_ => Ok(()),
	}
}

async fn remove_dir(path: &Path) -> std::io::Result<()> {
	match tokio::fs::remove_dir_all(path).await {
		Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
		_ => Ok(()),
	}
}

/// Makes the preview folders, then starts the prune.
///
/// The folders are made before the server answers its health check, so the
/// render worker never finds the requests folder missing and every folder is
/// this server's uid's (as `call_weeks::start` does).
pub fn start(state: &AppState) {
	for folder in [REQUESTS_FOLDER, RENDERS_FOLDER, STATUS_FOLDER, WORK_FOLDER] {
		if std::fs::create_dir_all(folder).is_err() {
			error!(error = "preview folder", "previews cannot make their folders");
		}
	}
	let cancel = state.core.cancel_token.clone();
	tokio::spawn(async move {
		info!(every_secs = PRUNE_EVERY.as_secs(), "preview prune started");
		let folders = Folders::home();
		let mut ticker = tokio::time::interval(PRUNE_EVERY);
		ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		loop {
			tokio::select! {
				() = cancel.cancelled() => {
					info!("preview prune cancelled");
					return;
				}
				_ = ticker.tick() => match prune(&folders, SystemTime::now()).await {
					Ok(pruned) if !(pruned.removed.is_empty() && pruned.overdue.is_empty() && pruned.swept.is_empty()) => {
						info!(removed = pruned.removed.len(), overdue = pruned.overdue.len(), swept = pruned.swept.len(), "preview prune");
					}
					Ok(_) => {}
					Err(err) => warn!(kind = ?err.kind(), "preview prune could not finish; the next one tries again"),
				},
			}
		}
	});
}

#[cfg(test)]
mod tests;
