//! Make part of a render again: an episode's video, its thumbnail or its
//! description (paulgsc/some-ui's Studio › Renders › Couldn't finish, and an
//! episode's assets). One request for one family:
//!
//! ```text
//! POST /feeds/:feed/videos/:key/produce   { asset }   → a Step
//! GET  /feeds/:feed/videos/:key/thumbnail             → image/png
//! ```
//!
//! What home can make again is an [`Asset`], a closed union matched whole
//! here, and every asset goes the one way work reaches home's render worker:
//! through its folders, under the lane (`docs/home-compute.md`, invariants 1
//! and 6). Nothing here renders, starts a job or adds a clock; it only makes
//! work available, as a request written once a week has ended does.
//!
//! **Where everything is: one path function.** [`Again::place`] derives
//! every path an asset has (its file, the episode's folder, its ask, its
//! failed log and refusal, the log a Run again moves aside) from `(feed,
//! key, asset)`, and nothing else builds one: the produce answer, the list's
//! steps and the thumbnail's serving all go through it. A key is taken only
//! as one plain name ([`plain_key`]), and only for an episode home holds; a
//! file's name is the closed union's, never the request's.
//!
//! - **[`Asset::Video`]**, for a row on Couldn't finish:
//!   - a failure the render policy refused (`too-long`, `too-large`,
//!     `overflow`) would come out the same (invariant 7): `409`, with its
//!     reason; the worker tries it again by itself once what it was refused
//!     for changes;
//!   - a cards essay home refused for its media is looked at again as home
//!     holds its media now (`cards_essays::Recheck`): still refused is `409`
//!     with the items that stop it; otherwise the cards pass runs now rather
//!     than on the hour;
//!   - any other failure (a burnt render) has its `<key>.failed.log` moved
//!     aside to `<key>.again.log`, its time the ask's, so the worker's next
//!     look takes it. Never while a `.refused` is beside the log, whatever
//!     it says (one that appears as it is moved puts it back: the worker
//!     writes a refusal before its log), and for a cards essay only once it
//!     was given to the worker (home writes its own refusal's log a moment
//!     before its `.refused`).
//! - **[`Asset::Thumbnail`]** is made alone, beside a finished video, by the
//!   worker's `asset` job: one ask file per `(feed, key, asset)`
//!   (`<asset>-<feed>-<key>` in [`ASKS_FOLDER`]), written once, so asking
//!   again changes nothing. The worker draws it into its own work folder,
//!   puts it in place by a link that never replaces a file there, and takes
//!   the ask away. One the recorder refused (its `asset refused:` line) keeps
//!   its ask, its failed log and a refusal holding it until what the worker's
//!   hold sums changes (the render and speaker images, the worker, the
//!   recorders' flags, the brand files, the episode's own files); any other failure is
//!   tried again on the hour, or at once by asking, its log moved aside as a
//!   video's (`infra/render/render-calls.sh`, `asset`).
//! - **[`Asset::Description`]** is the render's own chapters, at its real
//!   shot times: made only with the video. Asked while the video is there
//!   it is `422`, and the video is never rendered again for it; asked
//!   before, it is the video's ask.
//!
//! **Present means done:** a made file is never made again here, and
//! nothing here writes into an episode's folder.
//!
//! **What it judges by** is what the board shows (`feeds::renders`,
//! [`seen_of`]): one shelf's read, and the job the lane holds.

use crate::cards_essays::{Recheck, Rechecked};
use crate::feeds::renders::{job_of, seen_of, Blocking, Folders, Job, Phase, Shelf};
use crate::feeds::Feed;
use serde::Serialize;
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};

/// Where `file_host` writes the worker's asks, as this server sees it: on the
/// host `${STORAGE}/renders/asks`, the worker's `$ROOT/renders/asks`.
pub const ASKS_FOLDER: &str = "/app/dbs/renders/asks";

/// Where the worker's `asset` job says how its asks went.
pub const ASSET_STATUS_FOLDER: &str = "/app/dbs/renders/asset-worker";

/// The worker's name for the job that makes an asset alone (`job.state`'s
/// `kind`).
pub const ASSET_JOB: &str = "asset";

/// An episode's thumbnail, beside its video (some-ui's
/// `tests/record/thumbnail.ts`, C1026).
pub const THUMBNAIL: &str = "thumbnail.png";

/// The most bytes a thumbnail is served: a 1280×720 PNG is well under it.
pub const THUMBNAIL_CEILING: u64 = 4 * 1024 * 1024;

/// The longest key taken.
pub const MAX_KEY: usize = 128;

/// Makes the asks folder and the asset job's status folder.
///
/// Before the server answers its health check, by this server's uid, which
/// the worker runs as too: the lane counts a status folder it cannot list as an attempt in
/// progress (`call_weeks::lane`), so a missing one would hold home busy. A
/// folder that cannot be made is logged.
pub fn start() {
	for folder in [ASKS_FOLDER, ASSET_STATUS_FOLDER] {
		if std::fs::create_dir_all(folder).is_err() {
			tracing::error!(error = "render folder", "run again cannot make the render worker's asks or asset folders");
		}
	}
}

/// What home can make again of an episode: closed, matched whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asset {
	/// Its video: a render, which makes its description too.
	Video,
	/// `thumbnail.png`, made alone from the episode's own folder.
	Thumbnail,
	/// `description.txt`, its chapter lines, made only with the video.
	Description,
}

impl Asset {
	/// Every asset, in the order the list says them.
	pub const ALL: [Self; 3] = [Self::Video, Self::Thumbnail, Self::Description];

	/// The asset a request names, or `None`.
	#[must_use]
	pub fn parse(name: &str) -> Option<Self> {
		match name {
			"video" => Some(Self::Video),
			"thumbnail" => Some(Self::Thumbnail),
			"description" => Some(Self::Description),
			_ => None,
		}
	}

	/// Its name on the wire.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Video => "video",
			Self::Thumbnail => "thumbnail",
			Self::Description => "description",
		}
	}
}

/// The board's shelf a feed's episodes are on.
#[must_use]
pub const fn shelf_of(feed: Feed) -> Shelf {
	match feed {
		Feed::CardsEssay => Shelf::CardsEssay,
		_ => Shelf::Feed(feed),
	}
}

/// Whether `key` is one plain name.
///
/// Runs of ASCII letters and digits joined by single `-`, at most
/// [`MAX_KEY`] long. So no `/`, `.`, `..`, space or
/// empty run: it is one path segment, and never a hidden one. Case is kept
/// (a report's `2026-W46`); which key is home's is the feed's record's.
#[must_use]
pub fn plain_key(key: &str) -> bool {
	!key.is_empty() && key.len() <= MAX_KEY && key.split('-').all(|run| !run.is_empty() && run.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// Every path one asset of one episode has: [`Again::place`]'s, the only
/// place one is built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
	/// The episode's folder in its feed's renders folder.
	pub episode: PathBuf,
	/// The asset's file there.
	pub file: PathBuf,
	/// The last failed attempt's log, and its refusal beside it.
	pub failed: PathBuf,
	pub refused: PathBuf,
	/// Where Run again moves a failed log aside, its time the ask's: the
	/// video's (and so a description's), or a thumbnail's in the asset job's
	/// status folder.
	pub again: Option<PathBuf>,
	/// A thumbnail's: its ask, for the worker's `asset` job.
	pub ask: Option<PathBuf>,
	/// A cards essay's: the essay as given to the worker, which a render
	/// needs before its failure is the worker's.
	pub given: Option<PathBuf>,
	/// The lane's job for it, as `job.state` names one.
	pub job_kind: &'static str,
	pub job_key: String,
}

/// What Run again reads and writes.
#[derive(Clone)]
pub struct Again {
	/// The board's folders: each feed's renders and status folders, and the
	/// lane's.
	pub folders: Folders,
	/// [`ASKS_FOLDER`].
	pub asks: PathBuf,
	/// [`ASSET_STATUS_FOLDER`].
	pub asset_status: PathBuf,
	/// Where cards essays are given to the worker
	/// (`cards_essays::ESSAYS_FOLDER`).
	pub cards_given: PathBuf,
	pub cards: Recheck,
}

impl axum::extract::FromRef<crate::AppState> for Again {
	fn from_ref(state: &crate::AppState) -> Self {
		Self {
			folders: Folders::server(),
			asks: PathBuf::from(ASKS_FOLDER),
			asset_status: PathBuf::from(ASSET_STATUS_FOLDER),
			cards_given: PathBuf::from(crate::cards_essays::ESSAYS_FOLDER),
			cards: state.cards_essays.recheck.clone(),
		}
	}
}

impl Again {
	/// Every path `asset` of `feed`'s `key` has, or `None` for a key that is
	/// not one plain name ([`plain_key`]) or a feed with no folders. The one
	/// path function: pure, and every path it gives is one plain name below
	/// the feed's renders or status folder, the asks folder or the asset
	/// job's status folder (a file beside a video one more below).
	#[must_use]
	pub fn place(&self, feed: Feed, key: &str, asset: Asset) -> Option<Place> {
		if !plain_key(key) {
			return None;
		}
		let on = shelf_of(feed);
		let (renders, status) = self.folders.dirs(on)?;
		let episode = renders.join(key);
		let given = (on == Shelf::CardsEssay).then(|| self.cards_given.join(String::from(key) + ".json"));
		let render = |file: &str| Place {
			file: episode.join(file),
			failed: status.join(String::from(key) + ".failed.log"),
			refused: status.join(String::from(key) + ".refused"),
			again: Some(status.join(String::from(key) + ".again.log")),
			ask: None,
			given: given.clone(),
			job_kind: on.as_str(),
			job_key: key.to_owned(),
			episode: episode.clone(),
		};
		Some(match asset {
			Asset::Video => render(feed.video()),
			Asset::Description => render(crate::feeds::videos::DESCRIPTION),
			Asset::Thumbnail => {
				let job_key = [Asset::Thumbnail.as_str(), "-", feed.as_str(), "-", key].concat();
				Place {
					file: episode.join(THUMBNAIL),
					failed: self.asset_status.join(job_key.clone() + ".failed.log"),
					refused: self.asset_status.join(job_key.clone() + ".refused"),
					again: Some(self.asset_status.join(job_key.clone() + ".again.log")),
					ask: Some(self.asks.join(&job_key)),
					given: None,
					job_kind: ASSET_JOB,
					job_key,
					episode: episode.clone(),
				}
			}
		})
	}
}

/// Where an asset is, on the owner's conveyor: asked, waiting its turn, in
/// the oven, then done or burnt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
	/// Not made, and not asked for.
	Missing,
	/// Asked for; the worker has not named a job for it yet.
	Asked,
	/// The lane's job, waiting for room; or an episode in line.
	Waiting,
	/// The lane's job, rendering (or paused, or being put in place).
	Rendering,
	Done,
	/// Its last attempt made nothing.
	Failed,
	/// A step before it still stops it: home's refusal, or no video yet.
	Blocked,
}

/// One asset's step on the conveyor, each time read from the drive.
///
/// When it was asked (the ask's time, while it is in flight), when its job began
/// (the lane's `job.state`, while it is the lane's), and when it ended (the
/// made file's, or the failed log's). No second store: once done or failed,
/// only `endedAt` is on home.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
	pub asset: &'static str,
	pub state: Stage,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub asked_at: Option<i64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub started_at: Option<i64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub ended_at: Option<i64>,
	/// Why it failed or is blocked: the board's `reason` (`too-long`,
	/// `too-large`, `overflow`, a home refusal's), `recorder` for an asset
	/// the recorder refused (held: asking again changes nothing), `no-video`, `made-with-video`,
	/// `unreadable`, or `settling` for a failure whose refusal is still being
	/// written.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub reason: Option<&'static str>,
	/// What a home refusal names, still stopping it.
	#[serde(flatten, skip_serializing_if = "Option::is_none")]
	pub blocking: Option<Blocking>,
}

impl Step {
	const fn at(asset: Asset, state: Stage) -> Self {
		Self {
			asset: asset.as_str(),
			state,
			asked_at: None,
			started_at: None,
			ended_at: None,
			reason: None,
			blocking: None,
		}
	}

	const fn why(mut self, reason: &'static str) -> Self {
		self.reason = Some(reason);
		self
	}
}

/// An episode's three assets, for the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Assets {
	pub video: Step,
	pub thumbnail: Step,
	pub description: Step,
}

/// How a produce answer is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
	/// `202`: home will make it.
	Accepted,
	/// `200`: it is made.
	Made,
	/// `409`: a step still stops it.
	Conflict,
	/// `422`: it is never made alone (a description beside its video).
	Unprocessable,
}

/// The render policy's refusals: tried again by the worker itself, never on
/// asking (`docs/home-compute.md`, invariant 7).
const POLICY: [&str; 3] = ["too-long", "too-large", "overflow"];

/// Whether `reason` is a cards essay's home refusal (`essay_media`).
fn is_home_refusal(reason: &str) -> bool {
	matches!(reason, "take-missing" | "take-unheard" | "snip-missing" | "snip-gone" | "clip-missing")
}

/// When `path` was last written, Unix seconds, if it is there (not
/// followed: the worker writes plain files, and a link is none of ours).
async fn written(path: &Path) -> Option<i64> {
	let meta = tokio::fs::symlink_metadata(path).await.ok()?;
	let at = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
	i64::try_from(at.as_secs()).ok()
}

/// Whether `path` is a plain file.
async fn is_file(path: &Path) -> bool {
	tokio::fs::symlink_metadata(path).await.is_ok_and(|meta| meta.is_file())
}

/// One episode's read: what its shelf says of it, and the lane's job.
struct Seen<'a> {
	made: Option<i64>,
	failed: Option<i64>,
	reason: Option<&'static str>,
	blocking: Option<Blocking>,
	job: Option<&'a Job>,
}

impl<'a> Seen<'a> {
	fn of(found: &crate::feeds::renders::ShelfSeen, key: &str, job: Option<&'a Job>) -> Self {
		Self {
			made: found.seen.videos.get(key).copied(),
			failed: found.seen.failed.get(key).map(|(at, _)| *at),
			reason: found.seen.reasons.get(key).copied(),
			blocking: found.seen.blocking.get(key).cloned(),
			job,
		}
	}
}

/// The lane's job when it is `place`'s.
fn job_for<'a>(job: Option<&'a Job>, place: &Place) -> Option<&'a Job> {
	job.filter(|job| job.kind == place.job_kind && job.key == place.job_key)
}

/// `asset`'s step, by what `seen` and the drive say.
async fn step(place: &Place, seen: &Seen<'_>, asset: Asset) -> Step {
	match asset {
		Asset::Video => video_step(place, seen).await,
		Asset::Thumbnail => {
			if seen.made.is_none() {
				return Step::at(asset, Stage::Blocked).why("no-video");
			}
			if is_file(&place.file).await {
				return Step {
					ended_at: written(&place.file).await,
					..Step::at(asset, Stage::Done)
				};
			}
			let Some(asked_at) = (match place.ask.as_deref() {
				Some(ask) => written(ask).await,
				None => None,
			}) else {
				return Step::at(asset, Stage::Missing);
			};
			// The lane's job first: a failure tried again is in the oven.
			if job_for(seen.job, place).is_some() {
				return in_flight(asset, place, seen.job, Some(asked_at));
			}
			if let Some(ended) = written(&place.failed).await {
				// Held when the recorder itself refused it (`.refused`,
				// `reason=recorder`); any other failure is tried again on the
				// hour, or at once by Run again.
				let held = tokio::fs::symlink_metadata(&place.refused).await.is_ok();
				let step = Step {
					asked_at: Some(asked_at),
					ended_at: Some(ended),
					..Step::at(asset, Stage::Failed)
				};
				return if held { step.why("recorder") } else { step };
			}
			let asked_at = match place.again.as_deref() {
				Some(again) => written(again).await.map_or(asked_at, |again| again.max(asked_at)),
				None => asked_at,
			};
			in_flight(asset, place, seen.job, Some(asked_at))
		}
		Asset::Description => {
			if is_file(&place.file).await {
				return Step {
					ended_at: written(&place.file).await,
					..Step::at(asset, Stage::Done)
				};
			}
			if seen.made.is_some() {
				// Never made alone: the list says so as produce answers it.
				return Step::at(asset, Stage::Blocked).why("made-with-video");
			}
			// Made only with the video: its step is the video's.
			Step {
				asset: asset.as_str(),
				..video_step(place, seen).await
			}
		}
	}
}

/// The video's step.
async fn video_step(place: &Place, seen: &Seen<'_>) -> Step {
	let asset = Asset::Video;
	if let Some(made) = seen.made {
		return Step {
			ended_at: Some(made),
			..Step::at(asset, Stage::Done)
		};
	}
	let asked_at = match place.again.as_deref() {
		Some(again) => written(again).await,
		None => None,
	};
	// The lane's job first: a failure tried again on its own (the hour's
	// clock, a hold lifted) is in the oven, not failed.
	if job_for(seen.job, place).is_some() {
		return in_flight(asset, place, seen.job, asked_at);
	}
	if let Some(failed) = seen.failed {
		let mut step = Step {
			ended_at: Some(failed),
			reason: seen.reason,
			..Step::at(asset, Stage::Failed)
		};
		if seen.reason.is_some_and(is_home_refusal) {
			step.state = Stage::Blocked;
			step.blocking.clone_from(&seen.blocking);
		}
		return step;
	}
	let step = in_flight(asset, place, seen.job, asked_at);
	// Held, not made and not failed, and never asked again: in line.
	if step.state == Stage::Asked && asked_at.is_none() {
		return Step::at(asset, Stage::Waiting);
	}
	step
}

/// A step in flight: the lane's job when it is this one, else asked.
fn in_flight(asset: Asset, place: &Place, job: Option<&Job>, asked_at: Option<i64>) -> Step {
	job_for(job, place).map_or_else(
		|| Step {
			asked_at,
			..Step::at(asset, Stage::Asked)
		},
		|job| Step {
			asked_at,
			started_at: Some(job.since),
			..Step::at(
				asset,
				match job.state {
					Phase::Waiting => Stage::Waiting,
					Phase::Rendering | Phase::Paused | Phase::Finishing => Stage::Rendering,
				},
			)
		},
	)
}

/// Why Run again could not answer.
#[derive(Debug)]
pub enum Failed {
	Storage(sqlx::Error),
	/// The drive refused moving a failed log aside or writing an ask.
	Drive(std::io::Error),
}

impl From<sqlx::Error> for Failed {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(err)
	}
}

/// Each episode `feed`'s shelf holds, with its three assets' steps, for the
/// list (`GET /feeds/:feed/videos`): one shelf's read, the lane's job, and a
/// few `stat`s an episode.
///
/// # Errors
/// A storage failure.
pub async fn assets_of(db: &SqlitePool, again: &Again, subject: &str, feed: Feed, now: i64) -> Result<std::collections::BTreeMap<String, Assets>, sqlx::Error> {
	let mut assets = std::collections::BTreeMap::new();
	let Some(found) = seen_of(db, &again.folders, subject, shelf_of(feed), now).await? else {
		return Ok(assets);
	};
	let job = job_of(&again.folders).await;
	for (key, _) in &found.seen.held {
		let seen = Seen::of(&found, key, job.as_ref());
		let (Some(video), Some(thumbnail), Some(description)) = (
			again.place(feed, key, Asset::Video),
			again.place(feed, key, Asset::Thumbnail),
			again.place(feed, key, Asset::Description),
		) else {
			continue;
		};
		assets.insert(
			key.clone(),
			Assets {
				video: step(&video, &seen, Asset::Video).await,
				thumbnail: step(&thumbnail, &seen, Asset::Thumbnail).await,
				description: step(&description, &seen, Asset::Description).await,
			},
		);
	}
	Ok(assets)
}

/// Asks home to make `asset` of `feed`'s `key`, as the module doc says:
/// `None` for an episode the board doesn't list.
///
/// # Errors
/// [`Failed`].
pub async fn produce(db: &SqlitePool, again: &Again, subject: &str, feed: Feed, key: &str, asset: Asset, now: i64) -> Result<Option<(Code, Step)>, Failed> {
	let Some(place) = again.place(feed, key, asset) else {
		return Ok(None);
	};
	let Some(found) = seen_of(db, &again.folders, subject, shelf_of(feed), now).await? else {
		return Ok(None);
	};
	if !found.seen.held.iter().any(|(held, _)| held == key) {
		return Ok(None);
	}
	let job = job_of(&again.folders).await;
	let seen = Seen::of(&found, key, job.as_ref());
	let current = step(&place, &seen, asset).await;
	Ok(Some(match (asset, current.state) {
		(_, Stage::Done) => (Code::Made, current),
		(_, Stage::Asked | Stage::Waiting | Stage::Rendering) => (Code::Accepted, current),
		// Never made alone, and the good video is never rendered for it.
		(Asset::Description, Stage::Blocked) if current.reason == Some("made-with-video") => (Code::Unprocessable, current),
		(Asset::Thumbnail, Stage::Missing) => ask(&place, asset, now).await?,
		// No video yet; and a thumbnail the recorder refused would come out
		// the same: the worker holds it until its image or inputs change.
		// (A video and a description are never missing.)
		(Asset::Thumbnail, Stage::Blocked) | (Asset::Video | Asset::Description, Stage::Missing) => (Code::Conflict, current),
		(Asset::Thumbnail, Stage::Failed) if current.reason == Some("recorder") => (Code::Conflict, current),
		(Asset::Video | Asset::Description, Stage::Blocked) => recheck(db, again, key, current).await?,
		(_, Stage::Failed) => burnt(&place, current, now).await?,
	}))
}

/// A thumbnail's ask, written once: a second is the first.
async fn ask(place: &Place, asset: Asset, now: i64) -> Result<(Code, Step), Failed> {
	let Some(ask) = place.ask.as_deref() else {
		return Ok((Code::Conflict, Step::at(asset, Stage::Missing)));
	};
	if let Some(folder) = ask.parent() {
		tokio::fs::create_dir_all(folder).await.map_err(Failed::Drive)?;
	}
	// Empty, so there is never half of one; `create_new`, so it is never
	// written over and its time stays the first ask's.
	match tokio::fs::OpenOptions::new().write(true).create_new(true).open(ask).await {
		Ok(_) => {}
		Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
		Err(err) => return Err(Failed::Drive(err)),
	}
	Ok((
		Code::Accepted,
		Step {
			asked_at: Some(written(ask).await.unwrap_or(now)),
			..Step::at(asset, Stage::Asked)
		},
	))
}

/// The asset a step names: one of [`Asset::ALL`], as it was built.
fn asset_of(step: &Step) -> Asset {
	Asset::parse(step.asset).unwrap_or(Asset::Video)
}

/// A cards essay home refused: still refused, or the pass runs now.
async fn recheck(db: &SqlitePool, again: &Again, key: &str, current: Step) -> Result<(Code, Step), Failed> {
	Ok(match again.cards.refusal(db, key).await? {
		Rechecked::Refused(refusal) => {
			let items: Vec<String> = refusal
				.items
				.iter()
				.filter(|item| crate::feeds::renders::item_id(item))
				.take(crate::cards_essays::pass::REFUSED_ITEMS)
				.cloned()
				.collect();
			let mut blocking = Blocking::of(items, refusal.items.len());
			// A take's name is where the essay plays it (`cards_essays::names`).
			if let Some(blocking) = blocking.as_mut().filter(|_| refusal.kind == crate::essay_media::Kind::Takes) {
				let body = {
					let mut conn = db.acquire().await?;
					cards_essay_repo::body(&mut conn, key).await?
				};
				if let Some(essay) = body.and_then(|body| serde_json::from_slice::<serde_json::Value>(&body).ok()) {
					blocking.item_names = crate::cards_essays::names::take_names(&essay, &blocking.items);
				}
			}
			(
				Code::Conflict,
				Step {
					reason: Some(refusal.reason()),
					blocking,
					..current
				},
			)
		}
		// The pass would refuse it as unreadable again: nothing to run.
		Rechecked::Unreadable => (Code::Conflict, Step { blocking: None, ..current }.why("unreadable")),
		Rechecked::Clear => {
			again.cards.poke();
			(Code::Accepted, Step::at(asset_of(&current), Stage::Asked))
		}
	})
}

/// A failed render: a policy refusal is held; a burnt one has its log moved
/// aside, its time the ask's, so the worker's next look takes it.
async fn burnt(place: &Place, current: Step, now: i64) -> Result<(Code, Step), Failed> {
	if current.reason.is_some_and(|reason| POLICY.contains(&reason)) {
		return Ok((Code::Conflict, current));
	}
	// A refusal of any shape beside the log is never moved (the worker
	// writes it before the log; one that appears as the log is moved puts
	// it back, below); nor a cards essay's not given, whose log is home's
	// own refusal a moment before its `.refused`.
	let given = match place.given.as_deref() {
		Some(given) => is_file(given).await,
		None => true,
	};
	let refused = tokio::fs::symlink_metadata(&place.refused).await.is_ok();
	let Some(again) = place.again.as_deref().filter(|_| given && !refused) else {
		return Ok((Code::Conflict, current.why("settling")));
	};
	match tokio::fs::rename(&place.failed, again).await {
		// The worker ended an attempt between the look and the move: what
		// was moved is that attempt's log, its refusal now beside where it
		// was. Put back, never taken for a burnt one.
		// Put back by a link, which never replaces a log there: one there is
		// the worker's newer attempt's, and what was moved is stale.
		Ok(()) if tokio::fs::symlink_metadata(&place.refused).await.is_ok() => {
			match tokio::fs::hard_link(again, &place.failed).await {
				Ok(()) => {}
				Err(err) if matches!(err.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::AlreadyExists) => {}
				Err(err) => return Err(Failed::Drive(err)),
			}
			match tokio::fs::remove_file(again).await {
				Ok(()) => {}
				Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
				Err(err) => return Err(Failed::Drive(err)),
			}
			return Ok((Code::Conflict, current.why("settling")));
		}
		Ok(()) => {
			// Its time is the ask's (the conveyor's `askedAt`). Gone already
			// (the worker's attempt began and ended): it was taken.
			let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(u64::try_from(now).unwrap_or(0));
			let path = again.to_path_buf();
			let stamped = tokio::task::spawn_blocking(move || std::fs::File::options().write(true).open(path)?.set_modified(at))
				.await
				.unwrap_or_else(|err| Err(std::io::Error::other(err)));
			match stamped {
				Ok(()) => {}
				Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
				Err(err) => return Err(Failed::Drive(err)),
			}
		}
		// Moved by a tap just before, or the worker's own attempt ended.
		Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
		Err(err) => return Err(Failed::Drive(err)),
	}
	Ok((
		Code::Accepted,
		Step {
			asked_at: Some(now),
			..Step::at(asset_of(&current), Stage::Asked)
		},
	))
}

/// The thumbnail of `feed`'s `key`, whole, if home holds it and it has one.
///
/// Read through [`Again::place`], a plain file only, at most
/// [`THUMBNAIL_CEILING`] bytes (one over it is none). The worker puts a
/// thumbnail in place whole, by a link, so a read never sees half of one.
///
/// # Errors
/// A storage failure, or the drive refusing a read of a file that is there.
pub async fn thumbnail(db: &SqlitePool, again: &Again, subject: &str, feed: Feed, key: &str) -> Result<Option<Vec<u8>>, Failed> {
	use tokio::io::AsyncReadExt;
	let Some(place) = again.place(feed, key, Asset::Thumbnail) else {
		return Ok(None);
	};
	let holds = {
		let mut conn = db.acquire().await?;
		feed.holds(&mut conn, subject, key).await?
	};
	if !holds || !is_file(&place.file).await {
		return Ok(None);
	}
	let file = match tokio::fs::File::open(&place.file).await {
		Ok(file) => file,
		Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
		Err(err) => return Err(Failed::Drive(err)),
	};
	let mut bytes = Vec::new();
	file.take(THUMBNAIL_CEILING + 1).read_to_end(&mut bytes).await.map_err(Failed::Drive)?;
	Ok(u64::try_from(bytes.len()).is_ok_and(|read| read <= THUMBNAIL_CEILING).then_some(bytes))
}

#[cfg(test)]
mod tests {
	//! Promise RA1 (the PR declaring it): every asset's path is derived in one
	//! place and never leaves its folder; nothing is half written, written
	//! over, piled up or made twice; what failed and would come out the same
	//! is held. With `infra/render/render-calls.test.sh`'s asset cases.
	//!
	//! Over a cards essays shelf in a temporary folder: every asset's branch
	//! is a shelf's, and the cards essays' has every kind of failure (the
	//! worker's, the policy's and home's). The family the owner named: for
	//! each asset, ask, ask again, a crash mid-write, a worker restart and a
	//! read while it is written; and where each path may point.

	use super::*;
	use sqlx::sqlite::SqlitePoolOptions;

	const NOW: i64 = 1_763_600_000;

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		crate::schema::MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn again(dir: &Path) -> Again {
		Again {
			folders: Folders {
				feeds: Vec::new(),
				clip_walks: None,
				previews: None,
				cards_essays: Some((dir.join("renders"), dir.join("status"))),
				lane: dir.join("lane"),
			},
			asks: dir.join("asks"),
			asset_status: dir.join("asset-worker"),
			cards_given: dir.join("given"),
			cards: Recheck::for_tests(None),
		}
	}

	/// A cards essay of `day` whose scenes show `shots`, given to the worker.
	async fn keep(pool: &SqlitePool, dir: &Path, key: &str, day: &str, shots: serde_json::Value) {
		let body = serde_json::json!({
			"schema": "some-ui.cut-list/2",
			"title": "A machine upgrades itself",
			"series": { "id": "cheat-sheet", "version": 1 },
			"stamp": { "sources": ["paulgsc/nixos@abc1234"], "date": day },
			"shots": shots,
		})
		.to_string();
		let mut conn = pool.acquire().await.unwrap();
		cards_essay_repo::keep(
			&mut conn,
			&cards_essay_repo::Essay {
				key,
				day,
				body: body.as_bytes(),
				taken_at: "2026-11-20T09:00:00Z",
			},
		)
		.await
		.unwrap();
		write(&dir.join("given").join(String::from(key) + ".json"), &body);
	}

	fn write(path: &Path, text: &str) {
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, text).unwrap();
	}

	fn mtime(path: &Path) -> i64 {
		let at = std::fs::metadata(path).unwrap().modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap();
		i64::try_from(at.as_secs()).unwrap()
	}

	async fn produce(db: &SqlitePool, again: &Again, key: &str, asset: Asset) -> Option<(Code, Step)> {
		super::produce(db, again, "", Feed::CardsEssay, key, asset, NOW).await.unwrap()
	}

	/// The one path function: a key that is not one plain name has no
	/// place, and every place is one plain name below its folder.
	#[test]
	fn every_path_is_derived_in_one_place_and_never_leaves_its_folder() {
		let dir = Path::new("/srv");
		let again = again(dir);
		for key in [
			"",
			"..",
			".",
			"a/b",
			"../x",
			"a\\b",
			".hidden",
			"a..b",
			"a.b",
			"-a",
			"a-",
			"a--b",
			"a b",
			"caf\u{e9}",
			"a\0b",
			"%2e%2e",
		] {
			for asset in Asset::ALL {
				assert_eq!(again.place(Feed::CardsEssay, key, asset), None, "{key:?}");
			}
		}
		assert_eq!(again.place(Feed::CardsEssay, &"a".repeat(MAX_KEY + 1), Asset::Video), None);
		assert!(again.place(Feed::CardsEssay, &"a".repeat(MAX_KEY), Asset::Video).is_some());
		// A feed with no folders here has no place.
		assert_eq!(again.place(Feed::Call, "2026-11-06", Asset::Video), None);

		// Case is kept, never folded: 2026-W46 and 2026-w46 are two places.
		let upper = again.place(Feed::CardsEssay, "zfs-2026-W46", Asset::Thumbnail).unwrap();
		let lower = again.place(Feed::CardsEssay, "zfs-2026-w46", Asset::Thumbnail).unwrap();
		assert_ne!(upper.file, lower.file);
		assert_ne!(upper.ask, lower.ask);

		let video = again.place(Feed::CardsEssay, "zfs-2026-11-02", Asset::Video).unwrap();
		assert_eq!(video.file, dir.join("renders/zfs-2026-11-02/cards.mp4"));
		assert_eq!(video.failed, dir.join("status/zfs-2026-11-02.failed.log"));
		assert_eq!(video.again, Some(dir.join("status/zfs-2026-11-02.again.log")));
		assert_eq!((video.job_kind, video.job_key.as_str()), ("cards-essay", "zfs-2026-11-02"));
		let description = again.place(Feed::CardsEssay, "zfs-2026-11-02", Asset::Description).unwrap();
		assert_eq!(description.file, dir.join("renders/zfs-2026-11-02/description.txt"));
		assert_eq!(description.failed, video.failed, "made with the video: its log is the video's");
		let thumbnail = again.place(Feed::CardsEssay, "zfs-2026-11-02", Asset::Thumbnail).unwrap();
		assert_eq!(thumbnail.file, dir.join("renders/zfs-2026-11-02/thumbnail.png"));
		assert_eq!(thumbnail.ask, Some(dir.join("asks/thumbnail-cards-essay-zfs-2026-11-02")));
		assert_eq!(thumbnail.failed, dir.join("asset-worker/thumbnail-cards-essay-zfs-2026-11-02.failed.log"));
		assert_eq!((thumbnail.job_kind, thumbnail.job_key.as_str()), ("asset", "thumbnail-cards-essay-zfs-2026-11-02"));
		for place in [video, description, thumbnail] {
			for path in [Some(&place.file), Some(&place.failed), Some(&place.refused), place.again.as_ref(), place.ask.as_ref()]
				.into_iter()
				.flatten()
			{
				assert!(path.starts_with(dir), "{path:?}");
				assert!(
					path
						.components()
						.all(|part| matches!(part, std::path::Component::RootDir | std::path::Component::Normal(_))),
					"{path:?}"
				);
			}
		}
	}

	#[tokio::test]
	async fn a_burnt_video_is_taken_once_and_a_refused_one_never_moves() {
		let (db, dir) = (pool().await, tempfile::tempdir().unwrap());
		let again = again(dir.path());
		let status = dir.path().join("status");
		for (key, day) in [
			("burnt-2026-11-18", "2026-11-18"),
			("made-2026-11-19", "2026-11-19"),
			("waiting-2026-11-20", "2026-11-20"),
			("long-2026-11-21", "2026-11-21"),
			("odd-2026-11-22", "2026-11-22"),
		] {
			keep(&db, dir.path(), key, day, serde_json::json!([])).await;
		}

		// Not an episode home holds; an odd key never reaches the drive.
		assert_eq!(produce(&db, &again, "never-2026-11-01", Asset::Video).await, None);
		assert_eq!(produce(&db, &again, "../burnt-2026-11-18", Asset::Video).await, None);

		// Burnt: its log moved aside once, its time the ask's.
		write(&status.join("burnt-2026-11-18.failed.log"), "the recorder exited 1\n");
		let (code, step) = produce(&db, &again, "burnt-2026-11-18", Asset::Video).await.unwrap();
		assert_eq!((code, step.state, step.asked_at), (Code::Accepted, Stage::Asked, Some(NOW)));
		assert!(!status.join("burnt-2026-11-18.failed.log").exists());
		assert_eq!(
			std::fs::read_to_string(status.join("burnt-2026-11-18.again.log")).unwrap(),
			"the recorder exited 1\n",
			"the log is kept, not lost"
		);
		assert_eq!(mtime(&status.join("burnt-2026-11-18.again.log")), NOW);
		// Asked again: the same answer, the first ask's time, nothing moved.
		let later = super::produce(&db, &again, "", Feed::CardsEssay, "burnt-2026-11-18", Asset::Video, NOW + 60)
			.await
			.unwrap()
			.unwrap();
		assert_eq!((later.0, later.1.state, later.1.asked_at), (Code::Accepted, Stage::Asked, Some(NOW)));
		// The worker names its job: waiting its turn, then in the oven.
		write(
			&dir.path().join("lane/job.state"),
			&[
				"kind=cards-essay\nkey=burnt-2026-11-18\nsince=",
				&(NOW + 90).to_string(),
				"\nstate=rendering\nstate_since=",
				&(NOW + 95).to_string(),
				"\nyields=0\n",
			]
			.concat(),
		);
		let (_, step) = produce(&db, &again, "burnt-2026-11-18", Asset::Video).await.unwrap();
		assert_eq!((step.state, step.asked_at, step.started_at), (Stage::Rendering, Some(NOW), Some(NOW + 90)));
		// A second failure of the same attempt: one log aside at most, and
		// asked again it replaces it (the worker took the older one away).
		std::fs::remove_file(dir.path().join("lane/job.state")).unwrap();

		// Waiting its turn: already in line, nothing written.
		let (code, step) = produce(&db, &again, "waiting-2026-11-20", Asset::Video).await.unwrap();
		assert_eq!((code, step.state, step.asked_at), (Code::Accepted, Stage::Waiting, None));
		assert!(!status.join("waiting-2026-11-20.again.log").exists());

		// Refused by the render policy: it would come out the same.
		write(&status.join("long-2026-11-21.failed.log"), "the video would run 24.4 min, past the policy's 15.0 min\n");
		write(&status.join("long-2026-11-21.refused"), "reason=too-long\nminutes=24.4\ncap=15.0\nheld=ab12\n");
		let (code, step) = produce(&db, &again, "long-2026-11-21", Asset::Video).await.unwrap();
		assert_eq!((code, step.state, step.reason), (Code::Conflict, Stage::Failed, Some("too-long")));
		assert!(status.join("long-2026-11-21.failed.log").exists(), "left as it was");

		// A refusal of no shape the board reads, or one half written (its log
		// first): never moved, said settling.
		write(&status.join("odd-2026-11-22.failed.log"), "x\n");
		write(&status.join("odd-2026-11-22.refused"), "reason=a-later-kind\n");
		let (code, step) = produce(&db, &again, "odd-2026-11-22", Asset::Video).await.unwrap();
		assert_eq!((code, step.reason), (Code::Conflict, Some("settling")));
		assert!(status.join("odd-2026-11-22.failed.log").exists() && !status.join("odd-2026-11-22.again.log").exists());
		// A cards essay's failed log before it was given is home's refusal
		// being written: never the worker's to take.
		std::fs::remove_file(status.join("odd-2026-11-22.refused")).unwrap();
		std::fs::remove_file(dir.path().join("given/odd-2026-11-22.json")).unwrap();
		let (code, step) = produce(&db, &again, "odd-2026-11-22", Asset::Video).await.unwrap();
		assert_eq!((code, step.reason), (Code::Conflict, Some("settling")));
		assert!(status.join("odd-2026-11-22.failed.log").exists());

		// Made: present means done, never made again.
		let made = dir.path().join("renders").join("made-2026-11-19");
		write(&made.join(crate::cards_essays::VIDEO), "mp4");
		let (code, step) = produce(&db, &again, "made-2026-11-19", Asset::Video).await.unwrap();
		assert_eq!((code, step.state), (Code::Made, Stage::Done));
		assert!(!again.cards.poked().await, "no pass was asked for");
	}

	#[tokio::test]
	async fn a_description_is_made_only_with_its_video() {
		let (db, dir) = (pool().await, tempfile::tempdir().unwrap());
		let again = again(dir.path());
		keep(&db, dir.path(), "made-2026-11-19", "2026-11-19", serde_json::json!([])).await;
		keep(&db, dir.path(), "burnt-2026-11-18", "2026-11-18", serde_json::json!([])).await;
		let made = dir.path().join("renders").join("made-2026-11-19");
		write(&made.join(crate::cards_essays::VIDEO), "mp4");
		let (code, step) = produce(&db, &again, "made-2026-11-19", Asset::Description).await.unwrap();
		assert_eq!((code, step.state, step.reason), (Code::Unprocessable, Stage::Blocked, Some("made-with-video")));
		assert!(std::fs::read_dir(dir.path().join("status")).is_err(), "nothing written: the good video stays");
		write(&made.join("description.txt"), "0:00 Start\n");
		assert_eq!(produce(&db, &again, "made-2026-11-19", Asset::Description).await.unwrap().0, Code::Made);
		// No video yet: asking for its description is asking for the video.
		write(&dir.path().join("status/burnt-2026-11-18.failed.log"), "the recorder exited 1\n");
		let (code, step) = produce(&db, &again, "burnt-2026-11-18", Asset::Description).await.unwrap();
		assert_eq!((code, step.asset, step.state), (Code::Accepted, "description", Stage::Asked));
		assert!(dir.path().join("status/burnt-2026-11-18.again.log").exists());
	}

	#[tokio::test]
	async fn a_thumbnail_is_asked_once_and_never_half_served() {
		let (db, dir) = (pool().await, tempfile::tempdir().unwrap());
		let again = again(dir.path());
		keep(&db, dir.path(), "made-2026-11-19", "2026-11-19", serde_json::json!([])).await;
		keep(&db, dir.path(), "waiting-2026-11-20", "2026-11-20", serde_json::json!([])).await;
		let thumb = |key: &'static str| {
			let (db, again) = (db.clone(), again.clone());
			async move { super::thumbnail(&db, &again, "", Feed::CardsEssay, key).await.unwrap() }
		};
		let ask = dir.path().join("asks/thumbnail-cards-essay-made-2026-11-19");

		// Before its video: nothing asked.
		let (code, step) = produce(&db, &again, "waiting-2026-11-20", Asset::Thumbnail).await.unwrap();
		assert_eq!((code, step.reason), (Code::Conflict, Some("no-video")));
		assert!(!dir.path().join("asks").exists());

		let made = dir.path().join("renders").join("made-2026-11-19");
		write(&made.join(crate::cards_essays::VIDEO), "mp4");
		let (code, step) = produce(&db, &again, "made-2026-11-19", Asset::Thumbnail).await.unwrap();
		assert_eq!((code, step.state), (Code::Accepted, Stage::Asked));
		assert_eq!(std::fs::read(&ask).unwrap(), b"", "an empty ask: never half of one");
		// Asked again: the one ask, its first time, never written over.
		filetime_set(&ask, NOW - 30);
		let (code, step) = produce(&db, &again, "made-2026-11-19", Asset::Thumbnail).await.unwrap();
		assert_eq!((code, step.state, step.asked_at), (Code::Accepted, Stage::Asked, Some(NOW - 30)));
		assert_eq!(std::fs::read_dir(dir.path().join("asks")).unwrap().count(), 1, "one ask per (feed, key, asset)");

		// The worker's job, then a crash mid-write: a partial beside the
		// video is never served and never done; the ask stands for the
		// worker's restart, which sweeps the partial.
		write(
			&dir.path().join("lane/job.state"),
			&[
				"kind=asset\nkey=thumbnail-cards-essay-made-2026-11-19\nsince=",
				&NOW.to_string(),
				"\nstate=rendering\nstate_since=",
				&NOW.to_string(),
				"\nyields=0\n",
			]
			.concat(),
		);
		let (_, step) = produce(&db, &again, "made-2026-11-19", Asset::Thumbnail).await.unwrap();
		assert_eq!((step.state, step.started_at), (Stage::Rendering, Some(NOW)));
		write(&made.join(".thumbnail.png.partial"), "half a pn");
		std::fs::remove_file(dir.path().join("lane/job.state")).unwrap();
		assert_eq!(thumb("made-2026-11-19").await, None, "a partial is never served");
		let (code, step) = produce(&db, &again, "made-2026-11-19", Asset::Thumbnail).await.unwrap();
		assert_eq!((code, step.state), (Code::Accepted, Stage::Asked), "still asked: the worker takes it again");

		// It failed of something that may clear (a crash): tried again on
		// the hour, or now on asking, its log moved aside, its ask kept.
		let status = dir.path().join("asset-worker");
		write(&status.join("thumbnail-cards-essay-made-2026-11-19.failed.log"), "the recorder exited 137\n");
		let (code, step) = produce(&db, &again, "made-2026-11-19", Asset::Thumbnail).await.unwrap();
		assert_eq!((code, step.state, step.asked_at), (Code::Accepted, Stage::Asked, Some(NOW)));
		assert!(!status.join("thumbnail-cards-essay-made-2026-11-19.failed.log").exists());
		assert!(status.join("thumbnail-cards-essay-made-2026-11-19.again.log").exists() && ask.exists());
		// The recorder refused it (its exit 1, held by the worker): asking
		// again changes nothing, and nothing is moved.
		write(&status.join("thumbnail-cards-essay-made-2026-11-19.failed.log"), "the title fits no size\n");
		write(&status.join("thumbnail-cards-essay-made-2026-11-19.refused"), "reason=recorder\nheld=ab12\n");
		let (code, step) = produce(&db, &again, "made-2026-11-19", Asset::Thumbnail).await.unwrap();
		assert_eq!((code, step.state, step.reason), (Code::Conflict, Stage::Failed, Some("recorder")));
		assert!(status.join("thumbnail-cards-essay-made-2026-11-19.failed.log").exists());

		// Made, whole: served; present means done.
		for name in ["failed.log", "refused", "again.log"] {
			std::fs::remove_file(status.join(String::from("thumbnail-cards-essay-made-2026-11-19.") + name)).unwrap();
		}
		std::fs::remove_file(&ask).unwrap();
		write(&made.join(THUMBNAIL), "\u{89}PNG");
		assert_eq!(thumb("made-2026-11-19").await, Some("\u{89}PNG".as_bytes().to_vec()));
		let (code, step) = produce(&db, &again, "made-2026-11-19", Asset::Thumbnail).await.unwrap();
		assert_eq!((code, step.state), (Code::Made, Stage::Done));
		assert!(!ask.exists(), "never asked again once made");
		// Not an episode home holds, or a thumbnail that is no plain file.
		assert_eq!(thumb("never-2026-11-01").await, None);
		std::fs::remove_file(made.join(THUMBNAIL)).unwrap();
		std::os::unix::fs::symlink("/etc/hostname", made.join(THUMBNAIL)).unwrap();
		assert_eq!(thumb("made-2026-11-19").await, None, "a link is never followed");

		// The list says each asset's step.
		let listed = super::assets_of(&db, &again, "", Feed::CardsEssay, NOW).await.unwrap();
		assert_eq!(listed["made-2026-11-19"].video.state, Stage::Done);
		let reason = |step: &Step| (step.state, step.reason);
		assert_eq!(
			reason(&listed["made-2026-11-19"].description),
			(Stage::Blocked, Some("made-with-video")),
			"as produce answers it"
		);
		assert_eq!(reason(&listed["waiting-2026-11-20"].thumbnail), (Stage::Blocked, Some("no-video")), "as produce answers it");
		assert_eq!(listed["waiting-2026-11-20"].video.state, Stage::Waiting);
	}

	#[tokio::test]
	async fn a_cards_essay_home_refused_runs_the_pass_now_unless_it_is_still_refused() {
		let (db, dir) = (pool().await, tempfile::tempdir().unwrap());
		let again = again(dir.path());
		let status = dir.path().join("status");
		// Said refused for a take an hour ago; it names none now.
		keep(&db, dir.path(), "cleared-2026-11-18", "2026-11-18", serde_json::json!([])).await;
		write(&status.join("cleared-2026-11-18.failed.log"), "home refused this essay before the render: …: take-1\n");
		write(&status.join("cleared-2026-11-18.refused"), "reason=take-missing\nitems=take-1\ncount=1\n");
		// A clip frame home doesn't hold: refused still, and says which.
		let frame = serde_json::json!({ "picture": { "kind": "screen", "of": { "from": "clips", "clip": "studio-promo", "side": "after", "file": "01.png" } } });
		keep(&db, dir.path(), "framed-2026-11-20", "2026-11-20", serde_json::json!([frame])).await;
		write(
			&status.join("framed-2026-11-20.failed.log"),
			"home refused this essay before the render: …: studio-promo/after/01.png\n",
		);
		write(&status.join("framed-2026-11-20.refused"), "reason=clip-missing\nitems=studio-promo/after/01.png\ncount=1\n");

		let (code, step) = produce(&db, &again, "framed-2026-11-20", Asset::Video).await.unwrap();
		assert_eq!((code, step.state, step.reason), (Code::Conflict, Stage::Blocked, Some("clip-missing")));
		assert_eq!(step.blocking, Blocking::of(vec![String::from("studio-promo/after/01.png")], 1));
		assert!(!again.cards.poked().await, "still refused: no pass");
		assert!(status.join("framed-2026-11-20.failed.log").exists(), "home's refusal is the pass's to clear");

		let (code, step) = produce(&db, &again, "cleared-2026-11-18", Asset::Video).await.unwrap();
		assert_eq!((code, step.state), (Code::Accepted, Stage::Asked));
		assert!(again.cards.poked().await, "the pass runs now");
		assert!(status.join("cleared-2026-11-18.failed.log").exists(), "the pass clears it as it gives it");
	}

	#[test]
	fn an_asset_is_one_of_three_and_a_step_says_its_conveyor() {
		for name in ["video", "thumbnail", "description"] {
			assert_eq!(Asset::parse(name).map(Asset::as_str), Some(name));
		}
		assert_eq!(Asset::parse("chapters"), None);
		let step = Step {
			asked_at: Some(1),
			started_at: Some(2),
			blocking: Blocking::of(vec![String::from("note-a")], 3),
			..Step::at(Asset::Video, Stage::Blocked).why("take-unheard")
		};
		assert_eq!(
			serde_json::to_value(step).unwrap(),
			serde_json::json!({ "asset": "video", "state": "blocked", "askedAt": 1, "startedAt": 2, "reason": "take-unheard", "items": ["note-a"], "itemCount": 3 })
		);
		assert_eq!(
			serde_json::to_value(Step::at(Asset::Thumbnail, Stage::Missing)).unwrap(),
			serde_json::json!({ "asset": "thumbnail", "state": "missing" })
		);
	}

	/// Sets `path`'s time to `at`.
	fn filetime_set(path: &Path, at: i64) {
		let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(u64::try_from(at).unwrap());
		std::fs::File::options().write(true).open(path).unwrap().set_modified(at).unwrap();
	}
}
