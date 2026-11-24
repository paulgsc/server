//! Home writes each ended week's engagement report for its render worker
//! (paulgsc/some-ui ENG12, #2700, step 4; paulgsc/server#821).
//!
//! Once a week has ended on the owner's clock (`app_event_repo::ZONE`), this
//! worker reads the owner's engagement for it with `app_event_repo::weekly`,
//! the body `GET /app-events/weeks/:week` answers (#819), and keeps it in
//! `report_week` (`app_event_repo::report`), once. It then gives every week
//! home holds to the render worker as `<week>.json` in [`REPORTS_FOLDER`],
//! byte for byte as kept, written in a temporary file and renamed into place
//! whole. The render worker's `report` job (`infra/render/render-calls.sh`)
//! writes the report from it by code
//! (`record:report`: `reportWeek` under the owner's public-safety rule,
//! paulgsc/some-ui#2910, #2915), with no model and no GitHub leg anywhere,
//! so the same records give the same video.
//!
//! **Only a week that has ended, and only once** (C191: the recorder refuses
//! a week still under way). The weeks asked are the [`LOOKBACK`] whole
//! weeks ending with the last one ([`last_ended`]) that home does not hold
//! (C573), so a week a down home or a failing pass missed is still written
//! once home is back; each is asked only from [`GRACE`] after its end, so
//! what the phone took on Sunday evening and sent home on its next open is
//! in it. Once a week is kept it is never read again: events that reach home
//! later are in the next weeks' comparisons, never in this one's report, and
//! a re-render reads the same bytes. A week with no engagement record before
//! its end (`measured: false`) is not written: the report needs this week
//! measured, and an unmeasured week is "no data", never a report of zeros.
//! It is asked again each pass while it is within the lookback, so records
//! sent home late can still make it.
//!
//! **Whose.** The owner's account (`auth::owner`). With no account, every
//! week on the drive goes, its video, work and the worker's logs of it too
//! ([`tidy`]); with several, whose is not known, so nothing is written, and
//! a week no account holds goes as with none. Each write
//! of `report_week` holds the deletion lock across its check that the
//! account stands (docs/identity.md invariant 9), and a week on the drive
//! that the owner no longer holds leaves within the minute.
//!
//! **Not batch work.** A pass is a few indexed reads and one small file; the
//! render is the batch work, and it runs on home's lane like every other job
//! (docs/home-compute.md).
//!
//! **What it says.** When each pass started, whether it or the minute's tidy
//! failed, how many weeks home holds and whether the last whole week had no
//! records to report; each minute, what the render worker has made of the
//! weeks given ([`observe`], as every job's, `call_weeks::render::tally`)
//! (`metrics::report_weeks`): the CALL dashboard's "weekly engagement
//! report" row and the `ReportWeeksStale`, `ReportWeeksFailing`,
//! `ReportRenderFailed` and `ReportRenderOverdue` alerts in
//! `infra/prometheus/rules/home.yml`.
//!
//! **Kept for the phone, then let go.** The owner watches each week's video
//! on the phone, records its `YouTube` link and lets home's copy go from My
//! data, the video first, then the rest of its folder (`feeds::Feed::Report`,
//! `feeds::videos`), with a receipt each step (`report_video_deletion`). The
//! video's step takes the week's file out of [`REPORTS_FOLDER`] first, and a
//! week let go is never written there again, so it is never rendered again;
//! the week itself stays in `report_week`.
//!
//! Cost: every [`INTERVAL`], one read of the weeks held and, for each of the
//! [`LOOKBACK`] weeks home does not hold, its weekly read (four range reads
//! of `app_event_by_family` and one grouped read, then the friction
//! measures: one read joined against 28 day edges, one range read of the
//! window's `left`s with an indexed lookup each, at most `WINDOW_READ`, and
//! one range read of `jobs_event_order`), after one indexed read
//! of whether it is held; each minute, four `read_dir`s, and for what the
//! render worker made, one read of the weeks held and of those let go and up
//! to two `stat`s per week held.

use crate::auth::owner::Owner;
use crate::auth::DeletionLock;
use crate::call_weeks::render::{tally, Held, Renders};
use crate::AppState;
use app_event_repo::report::{self, Kept};
use app_event_repo::{IsoWeek, ZONE};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{error, info, warn};

/// Where each week home gives the render worker is written: on the host,
/// `${STORAGE}/report-weeks`, mounted read-only into the worker as
/// `/report-weeks` (`infra/compose/call-render.yml`).
pub const REPORTS_FOLDER: &str = "/app/dbs/report-weeks";

/// Where the worker keeps each week's video, as this server sees it.
pub const RENDERS_FOLDER: &str = "/app/dbs/renders/report";

/// Where the worker says how its passes went, as this server sees it.
pub const STATUS_FOLDER: &str = "/app/dbs/renders/report-worker";

/// Where the worker keeps a waiting week's work and step cache between
/// attempts, as this server sees it.
pub const WORK_FOLDER: &str = "/app/dbs/renders/report-work";

/// The video's file in a week's folder under [`RENDERS_FOLDER`].
pub const VIDEO: &str = "report.mp4";

/// How often home looks for a week to write: one ends a week.
pub const INTERVAL: Duration = Duration::from_secs(3600);

/// How often the drive is brought in line with what home holds.
pub const TICK: Duration = Duration::from_secs(60);

/// How long after its end a week is first asked: a day, so the phone's
/// Sunday-evening records, sent home when it next opens, are in it.
pub const GRACE: Duration = Duration::from_secs(24 * 3600);

/// How many whole weeks each pass asks, the last whole week among them.
///
/// A week home missed (down, or failing) for up to this many weeks is still
/// written once it is back (C573). Bounded, so a pass is at most this many
/// weekly reads, and a week older than this is never asked.
pub const LOOKBACK: i64 = 4;

const _: () = assert!(TICK.as_secs() <= INTERVAL.as_secs());

/// Start the worker.
///
/// Its folder and the render worker's are made first, before the server
/// answers its health check, by this server's uid, which the worker runs as
/// too, so the worker never finds one missing or not its own.
pub fn start(state: &AppState) {
	for folder in [REPORTS_FOLDER, RENDERS_FOLDER, WORK_FOLDER, STATUS_FOLDER] {
		if std::fs::create_dir_all(folder).is_err() {
			error!(error = "render folder", "report weeks worker cannot make the render worker's folders");
		}
	}
	let cancel = state.core.cancel_token.clone();
	let db = state.core.shared_db.clone();
	let deletion = state.auth.deletion_lock();
	tokio::spawn(async move {
		info!(interval_secs = INTERVAL.as_secs(), "report weeks worker started");
		crate::metrics::report_weeks::record_started();
		let folders = Folders::server();
		let mut ticker = tokio::time::interval(TICK);
		ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		let mut last: Option<tokio::time::Instant> = None;
		loop {
			tokio::select! {
				() = cancel.cancelled() => {
					info!("report weeks worker cancelled");
					return;
				}
				_ = ticker.tick() => {
					observe_renders(&db).await;
					let owner = crate::auth::owner::owner(&db).await;
					let tidied = if let Ok(owner) = &owner {
						tidy(&db, owner, &folders).await
					} else {
						error!(error = "storage", "report weeks cannot read the owner");
						Err(sqlx::Error::Protocol(String::from("the owner does not read")))
					};
					crate::metrics::report_weeks::record_tidy(&tidied);
					match tidied {
						Ok(tidied) if tidied.failed > 0 => warn!(failed = tidied.failed, "report weeks: files that could not be removed are tried again next minute"),
						Ok(_) => {}
						Err(_) if owner.is_err() => {}
						Err(_) => error!(error = "storage", "report weeks tidy failed"),
					}
					if last.is_some_and(|last| last.elapsed() < INTERVAL) {
						continue;
					}
					last = Some(tokio::time::Instant::now());
					crate::metrics::report_weeks::record_pass_started();
					let result = match owner {
						Ok(owner) => run_once(&db, &deletion, &owner, &folders, Utc::now()).await,
						Err(err) => Err(err),
					};
					crate::metrics::report_weeks::record_pass(&result);
					log_pass(&result);
				}
			}
		}
	});
}

/// Where the worker reads and writes: the server's folders, a temporary one
/// in tests.
#[derive(Debug, Clone)]
pub struct Folders {
	/// [`REPORTS_FOLDER`]: what the render worker is given.
	pub reports: PathBuf,
	/// [`RENDERS_FOLDER`]: what it made.
	pub renders: PathBuf,
	/// [`WORK_FOLDER`]: a waiting week's work and step cache.
	pub work: PathBuf,
	/// [`STATUS_FOLDER`]: the worker's logs of each week's attempts.
	pub status: PathBuf,
}

impl Folders {
	/// The folders, as this server sees them.
	#[must_use]
	pub fn server() -> Self {
		Self {
			reports: PathBuf::from(REPORTS_FOLDER),
			renders: PathBuf::from(RENDERS_FOLDER),
			work: PathBuf::from(WORK_FOLDER),
			status: PathBuf::from(STATUS_FOLDER),
		}
	}
}

/// What a pass made of the last whole week.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asked {
	/// Kept now.
	Written,
	/// Already kept: not read again.
	Held,
	/// Home holds no engagement record before its end: nothing written, and
	/// asked again next pass.
	Unmeasured,
	/// Its end is less than [`GRACE`] ago: not asked yet.
	Waiting,
	/// No owner (no account, or several): nothing asked.
	NoOwner,
}

/// What [`run_once`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pass {
	/// The last whole week, `2026-W46`.
	pub week: String,
	/// What the pass made of it.
	pub asked: Asked,
	/// Weeks kept this pass, the last whole one or one before it.
	pub written: usize,
	/// Weeks the owner holds.
	pub held: usize,
	/// Weeks written to the drive for the render worker this pass.
	pub exported: usize,
	/// Weeks that could not be written to the drive; tried again next pass.
	pub failed: usize,
}

/// The last ISO week, on the owner's clock, that had ended [`GRACE`] before
/// `now`.
#[must_use]
pub fn last_ended(now: DateTime<Utc>) -> IsoWeek {
	let asked = now - chrono::Duration::from_std(GRACE).unwrap_or_else(|_| chrono::Duration::days(1));
	IsoWeek::containing(asked.with_timezone(&ZONE).date_naive()).back(1)
}

/// One pass of the worker (the module doc).
///
/// It keeps each of the [`LOOKBACK`] whole weeks ending with the last one
/// that home does not hold and can (ended, measured), then writes
/// every week the owner holds that is not on the drive.
///
/// # Errors
/// A storage failure.
pub async fn run_once(db: &SqlitePool, deletion: &DeletionLock, owner: &Owner, folders: &Folders, now: DateTime<Utc>) -> Result<Pass, sqlx::Error> {
	let week = last_ended(now);
	let Owner::Subject(subject) = owner else {
		return Ok(Pass {
			week: week.label(),
			asked: Asked::NoOwner,
			written: 0,
			held: 0,
			exported: 0,
			failed: 0,
		});
	};
	let stamp = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
	// Oldest first, so the weeks are kept in their order; the last whole
	// week's answer is the pass's.
	let (mut asked, mut written) = (Asked::Waiting, 0);
	for back in (0..LOOKBACK).rev() {
		asked = ask(db, deletion, subject, week.back(back), &stamp).await?;
		written += usize::from(asked == Asked::Written);
	}
	let (held, exported, failed) = export(db, subject, folders).await?;
	Ok(Pass {
		week: week.label(),
		asked,
		written,
		held,
		exported,
		failed,
	})
}

/// Keeps `week` for `subject` if it is whole, measured and not held yet.
async fn ask(db: &SqlitePool, deletion: &DeletionLock, subject: &str, week: IsoWeek, now: &str) -> Result<Asked, sqlx::Error> {
	let weekly = {
		let mut conn = db.acquire().await?;
		if report::holds(&mut conn, subject, &week.label()).await? {
			return Ok(Asked::Held);
		}
		app_event_repo::weekly(&mut conn, subject, week, now).await.map_err(storage)?
	};
	let Some(this) = weekly.weeks.first() else {
		return Err(sqlx::Error::Protocol(String::from("the weekly read has no week")));
	};
	if !this.complete {
		// `last_ended` asks only a week that has ended: a clock that says
		// otherwise writes nothing (C191).
		return Ok(Asked::Waiting);
	}
	if !this.measured {
		return Ok(Asked::Unmeasured);
	}
	let mut body = Vec::new();
	serde_json::to_writer(&mut body, &weekly).map_err(|_| sqlx::Error::Protocol(String::from("the weekly read does not serialise")))?;
	let body = String::from_utf8(body).map_err(|_| sqlx::Error::Protocol(String::from("the weekly read is not UTF-8")))?;
	// The deletion lock first, then a connection, as every writer takes
	// them (docs/identity.md invariant 9): a deletion waiting for the lock
	// never waits on a connection this holds.
	let _hold = deletion.hold().await;
	let mut conn = db.acquire().await?;
	Ok(match report::keep(&mut conn, subject, week, &body, now).await? {
		Kept::Added => Asked::Written,
		Kept::Held => Asked::Held,
		Kept::NoAccount => Asked::NoOwner,
	})
}

fn storage(err: app_event_repo::EventError) -> sqlx::Error {
	match err {
		app_event_repo::EventError::Storage(err) => err,
		_ => sqlx::Error::Protocol(String::from("an engagement event does not read back")),
	}
}

/// Writes each week `subject` holds that is not on the drive, whole: the
/// weeks held, those written, and those that could not be.
async fn export(db: &SqlitePool, subject: &str, folders: &Folders) -> Result<(usize, usize, usize), sqlx::Error> {
	// Held for the whole export, as every feed's export holds it, so a step
	// that removes a week's files never runs between a read and a write.
	let _folders = crate::feeds::FOLDERS.lock().await;
	let mut conn = db.acquire().await?;
	let held = report::held(&mut conn, subject).await?;
	let let_go = app_event_repo::report_video::let_go(&mut conn, subject).await?;
	let (mut exported, mut failed) = (0, 0);
	for week in &held {
		// A week whose video was let go is never given again.
		if let_go.contains(week) {
			continue;
		}
		let into = file_of(&folders.reports, week);
		if tokio::fs::try_exists(&into).await.unwrap_or(false) {
			continue;
		}
		let Some(body) = report::body(&mut conn, subject, week).await? else { continue };
		if write_whole(&folders.reports, week, body.as_bytes()).await.is_ok() {
			exported += 1;
		} else {
			failed += 1;
			warn!(error = "storage", "a report week could not be written for the render worker");
		}
	}
	Ok((held.len(), exported, failed))
}

/// A week's file in the reports folder.
#[must_use]
pub fn file_of(reports: &Path, week: &str) -> PathBuf {
	reports.join(String::from(week) + ".json")
}

/// Writes `body` as `<week>.json`, through `.<week>.json.partial`, renamed
/// into place whole.
async fn write_whole(reports: &Path, week: &str, body: &[u8]) -> std::io::Result<()> {
	let partial = reports.join(String::from(".") + week + ".json.partial");
	tokio::fs::write(&partial, body).await?;
	tokio::fs::rename(&partial, file_of(reports, week)).await
}

/// What [`tidy`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Tidied {
	/// Weeks whose files went: no longer the owner's.
	pub weeks_removed: usize,
	/// Files that could not be removed; tried again next minute.
	pub failed: usize,
}

/// Each minute, the drive brought in line with what the owner holds.
///
/// A week on the drive the owner does not hold (their account was deleted,
/// or none stands) leaves it: its file, its video, its work and the worker's
/// logs of it (`<week>.failed.log`, `<week>.refused`, an attempt's
/// `.<week>.log`), and so does a `.partial` an export left half way. With
/// several accounts, whose is not known, so a week goes only once no account
/// holds it.
///
/// # Errors
/// A storage failure.
pub async fn tidy(db: &SqlitePool, owner: &Owner, folders: &Folders) -> Result<Tidied, sqlx::Error> {
	// The weeks held are read under the lock, so a week an export writes
	// between the read and the removal is never taken for one not held.
	let _folders = crate::feeds::FOLDERS.lock().await;
	let held = match owner {
		Owner::Subject(subject) => report::held(&mut *db.acquire().await?, subject).await?,
		Owner::NoAccount => Vec::new(),
		Owner::Several => report::held_by_anyone(&mut *db.acquire().await?).await?,
	};
	let mut done = Tidied::default();
	let mut on_drive = Vec::new();
	for (name, path) in entries(&folders.reports, &mut done.failed).await {
		if name.starts_with('.') && name.ends_with(".partial") {
			done.failed += usize::from(remove_file(&path).await.is_err());
		} else if let Some(week) = name.strip_suffix(".json").filter(|week| IsoWeek::parse(week).is_some()) {
			on_drive.push(week.to_owned());
		}
	}
	for dir in [&folders.renders, &folders.work] {
		for (name, _) in entries(dir, &mut done.failed).await {
			if IsoWeek::parse(&name).is_some() && !on_drive.contains(&name) {
				on_drive.push(name);
			}
		}
	}
	let mut logs = Vec::new();
	for (name, path) in entries(&folders.status, &mut done.failed).await {
		if let Some(week) = log_week(&name) {
			if !on_drive.contains(&week) {
				on_drive.push(week.clone());
			}
			logs.push((week, path));
		}
	}
	for week in on_drive {
		if held.contains(&week) {
			continue;
		}
		done.weeks_removed += 1;
		done.failed += usize::from(remove_file(&file_of(&folders.reports, &week)).await.is_err());
		for dir in [folders.renders.join(&week), folders.work.join(&week)] {
			done.failed += usize::from(crate::feeds::videos::remove_dir(&dir).await.is_err());
		}
		for (_, log) in logs.iter().filter(|(of, _)| *of == week) {
			done.failed += usize::from(remove_file(log).await.is_err());
		}
	}
	Ok(done)
}

/// Every entry of `dir`, by name and path. A folder that is not there has
/// none; one that does not read, or stops reading part way, counts one
/// failure in `failed`, so a tidy that could not see a week's files is a
/// failing tidy (`ReportWeeksFailing`), never a quiet one.
async fn entries(dir: &Path, failed: &mut usize) -> Vec<(String, PathBuf)> {
	let mut found = Vec::new();
	let mut read = match tokio::fs::read_dir(dir).await {
		Ok(read) => read,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => return found,
		Err(_) => {
			*failed += 1;
			return found;
		}
	};
	loop {
		match read.next_entry().await {
			Ok(Some(entry)) => found.push((entry.file_name().to_string_lossy().into_owned(), entry.path())),
			Ok(None) => return found,
			Err(_) => {
				*failed += 1;
				return found;
			}
		}
	}
}

/// The week a worker's file in the status folder is of: `<week>.<anything>`
/// or an attempt's `.<week>.<anything>`; `None` for its own files
/// (`last-pass`, `work.cache.log`).
fn log_week(name: &str) -> Option<String> {
	let name = name.strip_prefix('.').unwrap_or(name);
	let (week, rest) = (name.get(..8)?, name.get(8..)?);
	(rest.starts_with('.') && IsoWeek::parse(week).is_some()).then(|| week.to_owned())
}

/// What the worker has made of the weeks `owner` holds.
///
/// Each is let go, rendered (`<renders>/<week>/report.mp4`) or waiting, and
/// failed when `<status>/<week>.failed.log` is there. Nothing for no owner,
/// or several.
///
/// # Errors
/// A storage failure.
pub async fn observe(db: &SqlitePool, owner: &Owner, renders: &Path, status: &Path) -> Result<Renders, sqlx::Error> {
	let Owner::Subject(subject) = owner else {
		return Ok(tally(&[], &[], renders, status, VIDEO).await);
	};
	let (held, let_go) = held(db, subject).await?;
	Ok(tally(&held, &let_go, renders, status, VIDEO).await)
}

/// The weeks `subject` holds, each with when home wrote it, and those whose
/// video was let go, for [`observe`] and the phone's Renders page
/// (`feeds::renders`).
///
/// # Errors
/// A storage failure.
pub(crate) async fn held(db: &SqlitePool, subject: &str) -> Result<(Vec<Held>, Vec<String>), sqlx::Error> {
	let mut conn = db.acquire().await?;
	let held = report::kept(&mut conn, subject)
		.await?
		.into_iter()
		.map(|kept| Held {
			key: kept.week,
			taken_at: kept.written_at,
		})
		.collect();
	Ok((held, app_event_repo::report_video::let_go(&mut conn, subject).await?))
}

/// What the report's job takes of the drive (`call_weeks::render::disk`),
/// which the call's observation adds to its own, so the drive's numbers and
/// `CallRenderCacheFull` count every job.
pub async fn disk() -> crate::call_weeks::render::Disk {
	crate::call_weeks::render::disk(Path::new(RENDERS_FOLDER), Path::new(WORK_FOLDER), Path::new(STATUS_FOLDER)).await
}

/// What the render worker has made, as metrics. A failing read leaves the
/// last values and logs.
async fn observe_renders(db: &SqlitePool) {
	let seen = match crate::auth::owner::owner(db).await {
		Ok(owner) => observe(db, &owner, Path::new(RENDERS_FOLDER), Path::new(STATUS_FOLDER)).await,
		Err(err) => Err(err),
	};
	if let Ok(seen) = seen {
		crate::metrics::report_weeks::record_renders(&seen);
	} else {
		warn!(error = "storage", "report render observation failed");
	}
}

/// Removes a file; one already gone is fine.
async fn remove_file(path: &Path) -> std::io::Result<()> {
	match tokio::fs::remove_file(path).await {
		Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
		_ => Ok(()),
	}
}

/// The pass's one log line, when it wrote a week or something went wrong.
/// It names the week, never what it counts.
fn log_pass(result: &Result<Pass, sqlx::Error>) {
	match result {
		Ok(done) if done.written == 0 && done.exported == 0 && done.failed == 0 => {}
		Ok(done) if done.failed == 0 => info!(week = %done.week, asked = ?done.asked, written = done.written, held = done.held, exported = done.exported, "report weeks pass"),
		Ok(done) => warn!(week = %done.week, asked = ?done.asked, written = done.written, held = done.held, exported = done.exported, failed = done.failed, "report weeks pass"),
		Err(_) => error!(error = "storage", "report weeks pass failed"),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::{json, Value};
	use sqlx::sqlite::SqlitePoolOptions;

	const ME: &str = "subject-me";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		crate::schema::MIGRATOR.run(&pool).await.unwrap();
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?1)")
			.bind(ME)
			.execute(&pool)
			.await
			.unwrap();
		pool
	}

	fn folders(dir: &Path) -> Folders {
		let folders = Folders {
			reports: dir.join("report-weeks"),
			renders: dir.join("renders/report"),
			work: dir.join("renders/report-work"),
			status: dir.join("renders/report-worker"),
		};
		for folder in [&folders.reports, &folders.renders, &folders.work, &folders.status] {
			std::fs::create_dir_all(folder).unwrap();
		}
		folders
	}

	fn at(text: &str) -> DateTime<Utc> {
		DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc)
	}

	async fn taken(pool: &SqlitePool, id: &str, at: &str) {
		let event = json!({
			"family": "engagement", "id": id, "at": at, "build": null,
			"kind": "taken", "pipeline": "topik", "affordance": "routine-lens", "surface": "apk"
		});
		app_event_repo::put_event(&mut pool.acquire().await.unwrap(), ME, id, event.to_string().as_bytes(), at)
			.await
			.unwrap();
	}

	fn me() -> Owner {
		Owner::Subject(String::from(ME))
	}

	#[test]
	fn the_week_asked_is_the_last_whole_one_a_day_after_it_ended_on_the_owners_clock() {
		// 2026-W46 ends Monday 2026-11-16 at midnight in Los Angeles: 08:00 UTC.
		assert_eq!(last_ended(at("2026-11-17T07:59:59.000Z")).label(), "2026-W45", "less than a day after W46's end");
		assert_eq!(last_ended(at("2026-11-17T08:00:00.000Z")).label(), "2026-W46", "a day after it");
		assert_eq!(last_ended(at("2026-11-22T23:00:00.000Z")).label(), "2026-W46", "until a day after W47's end");
		assert_eq!(last_ended(at("2026-11-24T08:00:00.000Z")).label(), "2026-W47");
		// Across the new year: 2027-W01 starts Monday 2027-01-04.
		assert_eq!(last_ended(at("2027-01-05T08:00:00.000Z")).label(), "2026-W53");
	}

	#[tokio::test]
	async fn an_ended_measured_week_is_kept_once_and_given_byte_for_byte() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folders = folders(dir.path());
		let lock = DeletionLock::default();
		taken(&pool, "e1", "2026-11-10T09:00:00.000Z").await;
		let done = run_once(&pool, &lock, &me(), &folders, at("2026-11-17T09:00:00.000Z")).await.unwrap();
		assert_eq!(
			(done.week.as_str(), done.asked, done.held, done.exported, done.failed),
			("2026-W46", Asked::Written, 1, 1, 0)
		);
		let given = std::fs::read(file_of(&folders.reports, "2026-W46")).unwrap();
		let kept = report::body(&mut pool.acquire().await.unwrap(), ME, "2026-W46").await.unwrap().unwrap();
		assert_eq!(given, kept.as_bytes(), "the file is the kept body");
		let body: Value = serde_json::from_slice(&given).unwrap();
		assert_eq!(body["week"], "2026-W46");
		assert_eq!(body["weeks"][0]["complete"], true);
		assert_eq!(body["weeks"][0]["affordances"][0]["taken"], 1, "exactly the weekly read's body");

		// A late event of the week changes nothing: the week is not read again.
		taken(&pool, "e2", "2026-11-11T09:00:00.000Z").await;
		let again = run_once(&pool, &lock, &me(), &folders, at("2026-11-17T10:00:00.000Z")).await.unwrap();
		assert_eq!((again.asked, again.exported), (Asked::Held, 0));
		assert_eq!(std::fs::read(file_of(&folders.reports, "2026-W46")).unwrap(), given);

		// A file the drive lost is written again from what home kept.
		std::fs::remove_file(file_of(&folders.reports, "2026-W46")).unwrap();
		let restored = run_once(&pool, &lock, &me(), &folders, at("2026-11-17T11:00:00.000Z")).await.unwrap();
		assert_eq!(restored.exported, 1);
		assert_eq!(std::fs::read(file_of(&folders.reports, "2026-W46")).unwrap(), given);
		assert!(std::fs::read_dir(&folders.reports).unwrap().count() == 1, "no partial is left");
	}

	#[tokio::test]
	async fn a_week_a_down_home_missed_is_written_once_it_is_back_within_the_lookback() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folders = folders(dir.path());
		let lock = DeletionLock::default();
		for (id, at) in [("e1", "2026-10-20T09:00:00.000Z"), ("e2", "2026-10-27T09:00:00.000Z"), ("e3", "2026-11-10T09:00:00.000Z")] {
			taken(&pool, id, at).await;
		}
		// Home was down from W43's end until three weeks later: W43 to W46
		// are asked, oldest first, and W45, measured but empty, is a report too.
		let done = run_once(&pool, &lock, &me(), &folders, at("2026-11-17T09:00:00.000Z")).await.unwrap();
		assert_eq!((done.week.as_str(), done.asked, done.written, done.held), ("2026-W46", Asked::Written, 4, 4));
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(report::held(&mut conn, ME).await.unwrap(), ["2026-W43", "2026-W44", "2026-W45", "2026-W46"]);
		drop(conn);
		// A week older than the lookback is never asked.
		let (pool, dir) = (super::tests::pool().await, tempfile::tempdir().unwrap());
		let folders = super::tests::folders(dir.path());
		taken(&pool, "e1", "2026-10-13T09:00:00.000Z").await;
		let done = run_once(&pool, &lock, &me(), &folders, at("2026-11-17T09:00:00.000Z")).await.unwrap();
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(done.written, 4);
		assert!(!report::held(&mut conn, ME).await.unwrap().contains(&String::from("2026-W42")), "W42 is past the lookback");
	}

	#[tokio::test]
	async fn a_folder_that_does_not_read_is_a_failed_tidy_and_a_missing_one_is_none() {
		let dir = tempfile::tempdir().unwrap();
		let mut failed = 0;
		assert!(entries(&dir.path().join("missing"), &mut failed).await.is_empty());
		assert_eq!(failed, 0, "a folder not made yet is empty");
		// A file where a folder should be does not read as one.
		let file = dir.path().join("a-file");
		std::fs::write(&file, b"").unwrap();
		assert!(entries(&file, &mut failed).await.is_empty());
		assert_eq!(failed, 1);
	}

	#[test]
	fn a_workers_file_is_of_the_week_its_name_starts_with() {
		assert_eq!(log_week("2026-W46.failed.log").as_deref(), Some("2026-W46"));
		assert_eq!(log_week(".2026-W46.log").as_deref(), Some("2026-W46"));
		assert_eq!(log_week("2026-W46.refused").as_deref(), Some("2026-W46"));
		for not in ["last-pass", "work.cache.log", "2026-W4.log", "2026-W46", "2026-w46.log", ""] {
			assert_eq!(log_week(not), None, "{not}");
		}
	}

	#[tokio::test]
	async fn a_week_under_way_or_unmeasured_is_never_written() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folders = folders(dir.path());
		let lock = DeletionLock::default();
		// No record before W46's end: it is no data, not a report of zeros.
		let done = run_once(&pool, &lock, &me(), &folders, at("2026-11-17T09:00:00.000Z")).await.unwrap();
		assert_eq!((done.asked, done.held), (Asked::Unmeasured, 0));
		// A record of W47, which is under way: W46 is still unmeasured, and
		// W47 is not asked until a day after it ends.
		taken(&pool, "e1", "2026-11-18T09:00:00.000Z").await;
		let done = run_once(&pool, &lock, &me(), &folders, at("2026-11-23T09:00:00.000Z")).await.unwrap();
		assert_eq!((done.week.as_str(), done.asked, done.held), ("2026-W46", Asked::Unmeasured, 0));
		assert_eq!(std::fs::read_dir(&folders.reports).unwrap().count(), 0);
		// A record sent home late, of W46, makes it while W46 is the last whole week.
		taken(&pool, "e2", "2026-11-15T20:00:00.000Z").await;
		let done = run_once(&pool, &lock, &me(), &folders, at("2026-11-23T10:00:00.000Z")).await.unwrap();
		assert_eq!((done.week.as_str(), done.asked, done.held), ("2026-W46", Asked::Written, 1));
		let body: Value = serde_json::from_slice(&std::fs::read(file_of(&folders.reports, "2026-W46")).unwrap()).unwrap();
		assert_eq!(body["weeks"][0]["affordances"][0]["taken"], 1, "W47's record is not in W46");
	}

	#[tokio::test]
	async fn a_week_let_go_is_never_given_again_and_the_worker_is_read_by_its_files() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folders = folders(dir.path());
		let status = dir.path().join("renders/report-worker");
		std::fs::create_dir_all(&status).unwrap();
		let lock = DeletionLock::default();
		taken(&pool, "e1", "2026-11-03T09:00:00.000Z").await;
		run_once(&pool, &lock, &me(), &folders, at("2026-11-10T09:00:00.000Z")).await.unwrap();
		run_once(&pool, &lock, &me(), &folders, at("2026-11-17T09:00:00.000Z")).await.unwrap();
		let seen = observe(&pool, &me(), &folders.renders, &status).await.unwrap();
		assert_eq!((seen.held, seen.waiting, seen.failed, seen.let_go), (2, 2, 0, 0), "W45 and W46 wait for their videos");
		assert!(seen.oldest_waiting_taken.is_some());

		// W45 is made; W46's last attempt failed.
		std::fs::create_dir_all(folders.renders.join("2026-W45")).unwrap();
		std::fs::write(folders.renders.join("2026-W45").join(VIDEO), b"video").unwrap();
		std::fs::write(status.join("2026-W46.failed.log"), b"refused").unwrap();
		std::fs::write(status.join("last-pass"), b"1795400000\n").unwrap();
		let seen = observe(&pool, &me(), &folders.renders, &status).await.unwrap();
		assert_eq!((seen.waiting, seen.failed, seen.worker_last_pass), (1, 1, crate::call_weeks::render::Reading::Read(1_795_400_000)));

		// The owner lets W45's video go: its file leaves the input, and no
		// later pass writes it back.
		let mut conn = pool.acquire().await.unwrap();
		app_event_repo::report_video::record_link(&mut conn, ME, "2026-W45", "dQw4w9WgXcQ", "now").await.unwrap();
		let receipt = call_week_repo::video::Receipt {
			from: String::from("2026-W45"),
			step: call_week_repo::video::Step::Video,
			youtube_id: String::from("dQw4w9WgXcQ"),
			size: 5,
			deleted_at: String::from("now"),
		};
		app_event_repo::report_video::write_receipt(&mut conn, ME, &receipt).await.unwrap();
		drop(conn);
		std::fs::remove_file(file_of(&folders.reports, "2026-W45")).unwrap();
		let again = run_once(&pool, &lock, &me(), &folders, at("2026-11-17T10:00:00.000Z")).await.unwrap();
		assert_eq!((again.held, again.exported), (2, 0));
		assert!(!file_of(&folders.reports, "2026-W45").exists(), "never given again");
		let seen = observe(&pool, &me(), &folders.renders, &status).await.unwrap();
		assert_eq!((seen.held, seen.let_go, seen.waiting), (2, 1, 1));
		// No owner: nothing is counted as held.
		assert_eq!(observe(&pool, &Owner::NoAccount, &folders.renders, &status).await.unwrap().held, 0);
	}

	#[tokio::test]
	async fn no_owner_writes_nothing_and_a_week_no_longer_the_owners_leaves_the_drive() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folders = folders(dir.path());
		let lock = DeletionLock::default();
		taken(&pool, "e1", "2026-11-10T09:00:00.000Z").await;
		run_once(&pool, &lock, &me(), &folders, at("2026-11-17T09:00:00.000Z")).await.unwrap();
		for dir in [folders.renders.join("2026-W46"), folders.work.join("2026-W46")] {
			std::fs::create_dir_all(&dir).unwrap();
			std::fs::write(dir.join("report.mp4"), b"video").unwrap();
		}
		std::fs::write(folders.reports.join(".2026-W47.json.partial"), b"{").unwrap();
		std::fs::write(folders.reports.join("notes.txt"), b"not a week").unwrap();

		// Several accounts: whose is not known, so a week goes only once no
		// account holds it. ME's W46 stays; W45, held by nobody, goes, and so
		// does a half-written file.
		std::fs::write(file_of(&folders.reports, "2026-W45"), b"{}").unwrap();
		assert_eq!(tidy(&pool, &Owner::Several, &folders).await.unwrap(), Tidied { weeks_removed: 1, failed: 0 });
		assert!(file_of(&folders.reports, "2026-W46").exists() && !file_of(&folders.reports, "2026-W45").exists());
		assert!(!folders.reports.join(".2026-W47.json.partial").exists());
		std::fs::write(folders.reports.join(".2026-W47.json.partial"), b"{").unwrap();
		// The owner's own week stays; a half-written one goes.
		std::fs::write(folders.status.join("2026-W46.failed.log"), b"its attempt").unwrap();
		std::fs::write(folders.status.join(".2026-W46.log"), b"an attempt").unwrap();
		std::fs::write(folders.status.join("last-pass"), b"1").unwrap();
		assert_eq!(tidy(&pool, &me(), &folders).await.unwrap(), Tidied::default());
		assert!(file_of(&folders.reports, "2026-W46").exists() && folders.renders.join("2026-W46").exists());
		assert!(!folders.reports.join(".2026-W47.json.partial").exists());

		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(ME).execute(&pool).await.unwrap();
		let gone = run_once(&pool, &lock, &Owner::NoAccount, &folders, at("2026-11-17T10:00:00.000Z")).await.unwrap();
		assert_eq!((gone.asked, gone.held), (Asked::NoOwner, 0));
		assert_eq!(tidy(&pool, &Owner::NoAccount, &folders).await.unwrap(), Tidied { weeks_removed: 1, failed: 0 });
		assert!(!file_of(&folders.reports, "2026-W46").exists());
		assert!(
			!folders.renders.join("2026-W46").exists() && !folders.work.join("2026-W46").exists(),
			"its video and work go too"
		);
		assert!(folders.reports.join("notes.txt").exists(), "what is not a week is not touched");
		assert!(
			!folders.status.join("2026-W46.failed.log").exists() && !folders.status.join(".2026-W46.log").exists(),
			"nor its logs"
		);
		assert!(folders.status.join("last-pass").exists(), "the worker's own files stay");
		// A subject whose account went writes nothing, even asked by name
		// with a record of the week that outlived it.
		taken(&pool, "e2", "2026-11-11T09:00:00.000Z").await;
		let late = run_once(&pool, &lock, &me(), &folders, at("2026-11-17T11:00:00.000Z")).await.unwrap();
		assert_eq!((late.asked, late.held), (Asked::NoOwner, 0));
	}
}
