//! What home is rendering, what waits, what failed, what finished, and what
//! is coming: one read for the phone's Studio › Renders (SFEED13,
//! paulgsc/some-ui#2761).
//!
//! Everything here is state the render worker and the lane already keep on
//! the drive they share with this server (`infra/render/render-calls.sh`,
//! `docs/home-compute.md`, "What the render worker does"):
//!
//! - `renders/call-worker/job.state`: the job in progress, whichever feed's
//!   it is: its feed and episode, `waiting` / `rendering` / `paused`, since
//!   when, how often it gave way, and `ran`, the seconds it has rendered
//!   before its current state;
//! - `renders/call-worker/lane`: the lane's decision (`call_weeks::lane`),
//!   for why a job waits or is paused, and whether it is stuck;
//! - each feed's `took`: how long its last render rendered, pauses and waits
//!   left out, the rough bar's measure;
//! - each feed's renders folder (a video is an episode done) and status
//!   folder (`<key>.failed.log`, an episode that couldn't finish).
//!
//! **The estimate is an estimate.** Like an installer's bar: the share of the
//! last render's time this one has rendered, never past [`MOST_SHOWN`] until
//! it is done, with the time left at that pace. With no render measured
//! yet there is none, and the phone shows a bar that moves without a share.
//!
//! **What's coming** is each feed's cadence, not a schedule anyone keeps:
//! the call's next week (a week after the newest one home holds, on its
//! Friday) and Playoff Delta's next Tuesday, for a feed that took an episode
//! in the last [`ACTIVE_DAYS`] days, by home's clock ([`HOME_ZONE`]).
//!
//! **Cost:** the two feeds' held episodes (one read each, the tables grow by
//! an episode a week), a `stat` per episode, and a handful of small files,
//! once per request. Nothing here adds a clock or a poll: the phone asks.
//! It names no person; a failed attempt's line is the recorder's own words.

use super::Feed;
use crate::call_weeks::render::Held;
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc, Weekday};
use chrono_tz::Tz;
use serde::Serialize;
use sqlx::SqlitePool;
use std::collections::BTreeMap;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

/// Home's clock, for the day a feed's next episode is expected: the owner's
/// zone, as `config::DEFAULT_ROUTINE_SCHEDULE` names it.
pub const HOME_ZONE: Tz = chrono_tz::America::Los_Angeles;

/// A feed that took no episode for this long is not expecting one.
pub const ACTIVE_DAYS: i64 = 21;

/// The most of the bar shown before a render is done: an estimate never
/// says it has finished.
pub const MOST_SHOWN: f64 = 0.95;

/// How many finished renders are shown, newest first.
pub const DONE_SHOWN: usize = 5;

/// How much of a failed attempt's log is read, from its end.
const TAIL_BYTES: u64 = 16 * 1024;

/// The longest failed-attempt line shown, in characters.
const SAID_CHARS: usize = 200;

/// The job in progress, as the worker writes it to `job.state`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
	/// `call`, `playoff-delta`, or a kind a later worker adds (a preview).
	pub kind: String,
	pub key: String,
	/// Unix seconds when the job began.
	pub since: i64,
	pub state: Phase,
	/// Unix seconds when it reached its state.
	pub state_since: i64,
	/// How often it gave way.
	pub yields: u32,
	/// Seconds it rendered before `state_since`.
	pub ran: u64,
}

/// Where a job is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
	/// Waiting for room on home: before its first run, or after it gave way.
	Waiting,
	Rendering,
	/// Frozen while home's interactive work runs.
	Paused,
}

impl Job {
	/// The job `job.state`'s text names, or `None` when it names none whole.
	#[must_use]
	pub fn parse(text: &str) -> Option<Self> {
		let fields: BTreeMap<&str, &str> = text.lines().filter_map(|line| line.split_once('=')).collect();
		let field = |name: &str| fields.get(name).copied();
		let state = match field("state")? {
			"waiting" => Phase::Waiting,
			"rendering" => Phase::Rendering,
			"paused" => Phase::Paused,
			_ => return None,
		};
		let kind = field("kind")?;
		let key = field("key")?;
		let plain = |name: &str| !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
		if !plain(kind) || !plain(key) {
			return None;
		}
		Some(Self {
			kind: kind.to_owned(),
			key: key.to_owned(),
			since: field("since")?.parse().ok()?,
			state,
			state_since: field("state_since")?.parse().ok()?,
			yields: field("yields")?.parse().ok()?,
			ran: field("ran").map_or(Some(0), |ran| ran.parse().ok())?,
		})
	}

	/// Seconds it has rendered by `now`.
	#[must_use]
	pub fn ran_by(&self, now: i64) -> u64 {
		let running = if self.state == Phase::Rendering {
			u64::try_from(now.saturating_sub(self.state_since)).unwrap_or(0)
		} else {
			0
		};
		self.ran.saturating_add(running)
	}
}

/// The lane's decision, as `call_weeks::lane` writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lane {
	pub verb: String,
	pub why: String,
	pub stuck: bool,
	pub at: i64,
}

impl Lane {
	/// The decision `lane`'s text says, or `None` when it says none whole.
	#[must_use]
	pub fn parse(text: &str) -> Option<Self> {
		let fields: BTreeMap<&str, &str> = text.lines().filter_map(|line| line.split_once('=')).collect();
		Some(Self {
			verb: (*fields.get("verb")?).to_owned(),
			why: (*fields.get("why")?).to_owned(),
			stuck: *fields.get("stuck")? == "true",
			at: fields.get("at")?.parse().ok()?,
		})
	}
}

/// What one feed's folders say of its episodes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeedSeen {
	/// Each episode held and not let go, with when home took it (Unix
	/// seconds), oldest first.
	pub held: Vec<(String, i64)>,
	/// Of those, each with its video, and when it was made.
	pub videos: BTreeMap<String, i64>,
	/// Of those with none, each whose last attempt failed: when, and its line.
	pub failed: BTreeMap<String, (i64, String)>,
	/// The seconds its last render rendered.
	pub took: Option<u64>,
	/// The newest `from` home holds, the call's only: its weeks are days.
	pub newest_week: Option<NaiveDate>,
}

/// The read, as the phone gets it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Board {
	/// What home is rendering, or waiting for room to render, if anything.
	pub now: Option<Now>,
	/// Episodes in line after it, oldest first.
	pub waiting: Vec<Episode>,
	/// Episodes whose last attempt couldn't finish, newest first.
	pub failed: Vec<Failed>,
	/// The last few finished, newest first.
	pub done: Vec<Episode>,
	/// Each feed's next expected episode.
	pub coming: Vec<Coming>,
}

/// The job in progress.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Now {
	pub feed: String,
	pub key: String,
	pub state: Phase,
	/// Unix seconds when the job began.
	pub since: i64,
	/// Unix seconds when it reached its state.
	pub state_since: i64,
	/// How often it gave way and picked up again.
	pub gave_way: u32,
	/// Why it waits or is paused, as the lane said it (`busy`, `memory`,
	/// `pressure`, `settling`, `unknown`), or `silent` when the lane has
	/// stopped deciding; `None` while it renders.
	pub held_by: Option<String>,
	/// The lane has stopped it so often for memory that it may never fit.
	pub stuck: bool,
	pub estimate: Option<Estimate>,
}

/// A rough share of the render done, and the time left at its pace.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Estimate {
	/// 0 to [`MOST_SHOWN`].
	pub share: f64,
	/// `None` once the last render's time has passed: nearly done.
	pub seconds_left: Option<u64>,
}

/// An episode, and when (Unix seconds): taken, for one waiting; made, for
/// one done.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Episode {
	pub feed: String,
	pub key: String,
	pub at: i64,
}

/// An episode whose last attempt couldn't finish.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Failed {
	pub feed: String,
	pub key: String,
	pub at: i64,
	/// The recorder's last line of it, at most [`SAID_CHARS`] characters.
	pub said: String,
}

/// A feed's next expected episode, on its day by home's clock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Coming {
	pub feed: String,
	/// `YYYY-MM-DD`.
	pub on: String,
}

/// The estimate for `job`, given its feed's last render's time.
#[must_use]
pub fn estimate(job: &Job, took: Option<u64>, now: i64) -> Option<Estimate> {
	let took = took.filter(|took| *took > 0)?;
	let ran = job.ran_by(now);
	#[allow(clippy::cast_precision_loss)] // seconds, far under 2^52
	let share = (ran as f64 / took as f64).min(MOST_SHOWN);
	Some(Estimate {
		share,
		seconds_left: took.checked_sub(ran).filter(|left| *left > 0),
	})
}

/// Why `job` isn't rendering, by the lane's decision.
fn held_by(job: &Job, lane: Option<&Lane>, now: i64) -> Option<String> {
	if job.state == Phase::Rendering {
		return None;
	}
	let stale = i64::try_from(crate::call_weeks::lane::STALE_AFTER.as_secs()).unwrap_or(i64::MAX);
	match lane {
		Some(lane) if now.saturating_sub(lane.at).abs() <= stale => Some(lane.why.clone()),
		_ => Some(String::from("silent")),
	}
}

/// The first day `weekday` on or after `day`.
fn on_or_after(day: NaiveDate, weekday: Weekday) -> NaiveDate {
	let ahead = (7 + i64::from(weekday.num_days_from_monday()) - i64::from(day.weekday().num_days_from_monday())) % 7;
	day + Duration::days(ahead)
}

/// The day `feed`'s next episode is expected, if it is expecting one.
fn coming(feed: Feed, seen: &FeedSeen, now: i64, today: NaiveDate) -> Option<NaiveDate> {
	let newest_taken = seen.held.iter().map(|(_, taken)| *taken).max()?;
	if now.saturating_sub(newest_taken) > ACTIVE_DAYS * 86_400 {
		return None;
	}
	match feed {
		// A week after the newest week held, on its first day, a Friday.
		Feed::Call => {
			let next = seen.newest_week? + Duration::days(7);
			Some(if next >= today { next } else { on_or_after(today, next.weekday()) })
		}
		// The Tuesday after the newest fill was taken, today at the soonest.
		Feed::PlayoffDelta => {
			let taken = DateTime::<Utc>::from_timestamp(newest_taken, 0)?.with_timezone(&HOME_ZONE).date_naive();
			Some(on_or_after((taken + Duration::days(1)).max(today), Weekday::Tue))
		}
	}
}

/// The board, from what was read: pure, so every rule is tested.
#[must_use]
pub fn board(feeds: &[(Feed, FeedSeen)], job: Option<&Job>, lane: Option<&Lane>, now: i64) -> Board {
	let today = DateTime::<Utc>::from_timestamp(now, 0).map_or(NaiveDate::MIN, |at| at.with_timezone(&HOME_ZONE).date_naive());
	let now_view = job.map(|job| Now {
		feed: job.kind.clone(),
		key: job.key.clone(),
		state: job.state,
		since: job.since,
		state_since: job.state_since,
		gave_way: job.yields,
		held_by: held_by(job, lane, now),
		stuck: lane.is_some_and(|lane| lane.stuck),
		estimate: estimate(job, Feed::parse(&job.kind).and_then(|feed| feeds.iter().find(|(f, _)| *f == feed)).and_then(|(_, seen)| seen.took), now),
	});
	let mut waiting = Vec::new();
	let mut failed = Vec::new();
	let mut done = Vec::new();
	let mut expected = Vec::new();
	for (feed, seen) in feeds {
		let name = feed.as_str();
		for (key, taken) in &seen.held {
			let current = job.is_some_and(|job| job.kind == name && job.key == *key);
			if let Some(made) = seen.videos.get(key) {
				done.push(Episode { feed: name.to_owned(), key: key.clone(), at: *made });
			} else if let Some((at, said)) = seen.failed.get(key).filter(|_| !current) {
				failed.push(Failed {
					feed: name.to_owned(),
					key: key.clone(),
					at: *at,
					said: said.clone(),
				});
			} else if !current {
				waiting.push(Episode { feed: name.to_owned(), key: key.clone(), at: *taken });
			}
		}
		if let Some(on) = coming(*feed, seen, now, today) {
			expected.push(Coming {
				feed: name.to_owned(),
				on: on.format("%Y-%m-%d").to_string(),
			});
		}
	}
	// The worker's own order: the call's first, each feed's oldest first.
	waiting.sort_by_key(|episode| (episode.feed != Feed::Call.as_str(), episode.at));
	failed.sort_by_key(|failed| std::cmp::Reverse(failed.at));
	done.sort_by_key(|episode| std::cmp::Reverse(episode.at));
	done.truncate(DONE_SHOWN);
	expected.sort_by(|a, b| a.on.cmp(&b.on));
	Board {
		now: now_view,
		waiting,
		failed,
		done,
		coming: expected,
	}
}

/// Where the read looks: the real folders in the server, a temporary one in
/// tests.
#[derive(Debug, Clone)]
pub struct Folders {
	/// Each feed's renders and status folders.
	pub feeds: Vec<(Feed, PathBuf, PathBuf)>,
	/// The lane's folder: `job.state` and `lane`.
	pub lane: PathBuf,
}

impl Folders {
	#[must_use]
	pub fn server() -> Self {
		Self {
			feeds: vec![
				(Feed::Call, PathBuf::from(crate::call_weeks::render::RENDERS_FOLDER), PathBuf::from(crate::call_weeks::render::STATUS_FOLDER)),
				(
					Feed::PlayoffDelta,
					PathBuf::from(crate::pd_fills::render::RENDERS_FOLDER),
					PathBuf::from(crate::pd_fills::render::STATUS_FOLDER),
				),
			],
			lane: PathBuf::from(crate::call_weeks::render::STATUS_FOLDER),
		}
	}
}

/// Reads the board.
///
/// # Errors
/// A storage failure reading the feeds' episodes. A file that cannot be read
/// counts as missing.
pub async fn read(db: &SqlitePool, folders: &Folders, now: i64) -> Result<Board, sqlx::Error> {
	let mut feeds = Vec::with_capacity(folders.feeds.len());
	for (feed, renders, status) in &folders.feeds {
		let (held, let_go) = match feed {
			Feed::Call => crate::call_weeks::render::held(db).await?,
			Feed::PlayoffDelta => {
				let (_, voiced, let_go) = crate::pd_fills::render::voiced(db).await?;
				(voiced, let_go)
			}
		};
		feeds.push((*feed, seen(*feed, &held, &let_go, renders, status).await));
	}
	let job = small(&folders.lane.join("job.state")).await.as_deref().and_then(Job::parse);
	let lane = small(&folders.lane.join(crate::call_weeks::lane::DECISION)).await.as_deref().and_then(Lane::parse);
	Ok(board(&feeds, job.as_ref(), lane.as_ref(), now))
}

/// A small file's text, or `None`.
async fn small(path: &Path) -> Option<String> {
	tokio::fs::read_to_string(path).await.ok()
}

async fn seen(feed: Feed, held: &[Held], let_go: &[String], renders: &Path, status: &Path) -> FeedSeen {
	let mut seen = FeedSeen {
		took: small(&status.join("took")).await.and_then(|took| took.trim().parse().ok()),
		..FeedSeen::default()
	};
	for episode in held {
		if let_go.contains(&episode.key) {
			continue;
		}
		let taken = DateTime::parse_from_rfc3339(&episode.taken_at).map_or(0, |at| at.timestamp());
		seen.held.push((episode.key.clone(), taken));
		if feed == Feed::Call {
			if let Ok(day) = NaiveDate::parse_from_str(&episode.key, "%Y-%m-%d") {
				seen.newest_week = seen.newest_week.max(Some(day));
			}
		}
		if let Some(made) = modified(&renders.join(&episode.key).join(feed.video())).await {
			seen.videos.insert(episode.key.clone(), made);
			continue;
		}
		let mut name = episode.key.clone();
		name.push_str(".failed.log");
		let log = status.join(name);
		if let Some(at) = modified(&log).await {
			seen.failed.insert(episode.key.clone(), (at, last_said(&log).await.unwrap_or_default()));
		}
	}
	seen.held.sort_by_key(|(_, taken)| *taken);
	seen
}

/// When `path` was last written, Unix seconds, if it is there.
async fn modified(path: &Path) -> Option<i64> {
	let at = tokio::fs::metadata(path).await.ok()?.modified().ok()?;
	let since = at.duration_since(std::time::UNIX_EPOCH).ok()?;
	i64::try_from(since.as_secs()).ok()
}

/// The recorder's last line in a failed attempt's log: the worker's own
/// lines (that it waited, paused, gave way, and the exit it saw) left out,
/// so what is shown is what failed.
async fn last_said(log: &Path) -> Option<String> {
	let mut file = tokio::fs::File::open(log).await.ok()?;
	let len = file.metadata().await.ok()?.len();
	file.seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES))).await.ok()?;
	let mut bytes = Vec::new();
	file.read_to_end(&mut bytes).await.ok()?;
	Some(said_in(&String::from_utf8_lossy(&bytes)))
}

/// The last line of `text` that the recorder said, at most [`SAID_CHARS`].
#[must_use]
pub fn said_in(text: &str) -> String {
	let worker_own = |line: &str| {
		line.starts_with("the recorder exited")
			|| line.starts_with("paused the ")
			|| line.starts_with("resumed the ")
			|| line.contains(" waits for room on home ")
			|| line.contains(" gave way (the lane: ")
			|| line.starts_with("cache warning: ")
	};
	let lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
	let line = lines.clone().filter(|line| !worker_own(line)).last().or_else(|| lines.last()).unwrap_or_default();
	line.chars().take(SAID_CHARS).collect()
}

#[cfg(test)]
mod tests;
