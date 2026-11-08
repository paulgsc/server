//! What home's render worker has made of the weeks home holds, read from the
//! drive they share (paulgsc/some-ui#2635, CW5).
//!
//! The worker (`infra/render/render-calls.sh`) keeps each week's video in
//! `renders/call/<from>/call.mp4`, stamps each pass it starts in
//! `renders/call-worker/last-pass`, and keeps a week's last failed attempt in
//! `renders/call-worker/<from>.failed.log`, all under `STORAGE`, which this
//! server mounts at `/app/dbs` and the worker's `/renders` is a folder of
//! (`infra/compose/call-render.yml`). Every tick of the call weeks worker
//! reads those against `call_week` for the CALL dashboard and the
//! `CallRender*` alerts (`metrics::call_weeks::record_renders`).
//!
//! A week whose video the owner let go from My data (`videos::let_go`) is
//! counted apart, never as waiting (CW8).
//!
//! The same folder carries the batch lane's files (`super::lane`): `lane`,
//! the decision this server writes for the worker, and `job` and `peak`, the
//! job in progress and its measured peak, which the worker writes (L4c).
//!
//! It reads file names and one number, never a week's words or a log's.
//! Cost: one read of `call_week`'s days and up to two `stat`s per week held,
//! each minute; the table grows by a week a week.
//!
//! [`disk`] reads what the call takes of that drive, for the CALL dashboard
//! and the `CallRenderCacheFull` and `HomeDriveLow` alerts: the bytes of the
//! videos kept for good and of the waiting weeks' work (their step caches),
//! when the worker last wrote a cache warning (a step past the render
//! policy's budget, `renders/call-worker/*.cache.log`), and the drive's free
//! and total bytes. Cost: a `stat` per file under `renders/` (about eighty a
//! week rendered) and one `statvfs`, each minute.

use chrono::DateTime;
use sqlx::SqlitePool;
use std::path::Path;

/// Where the worker keeps each week's video, as this server sees it.
pub const RENDERS_FOLDER: &str = "/app/dbs/renders/call";

/// Where the worker says how its passes went, as this server sees it.
pub const STATUS_FOLDER: &str = "/app/dbs/renders/call-worker";

/// Where the worker keeps a waiting week's work and step cache between
/// attempts, as this server sees it.
pub const WORK_FOLDER: &str = "/app/dbs/renders/call-work";

/// What the call takes of the drive that keeps it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Disk {
	/// Bytes of every video kept for good, and what each was made from.
	pub videos_bytes: u64,
	/// Bytes of the waiting weeks' work and step caches.
	pub work_bytes: u64,
	/// Unix seconds of the newest cache warning, if one was ever written.
	pub cache_warning_at: Option<i64>,
	/// The drive's bytes free to the worker and its size, if it can be read.
	pub drive: Option<(u64, u64)>,
}

/// Reads what the call takes of the drive (the module doc).
pub async fn disk(renders: &Path, work: &Path, status: &Path) -> Disk {
	Disk {
		videos_bytes: bytes_under(renders).await,
		work_bytes: bytes_under(work).await,
		cache_warning_at: newest_cache_warning(status).await,
		drive: drive(renders.parent().unwrap_or(renders)),
	}
}

/// Every file's bytes under `dir`, not following links; nothing when it is
/// missing.
pub(crate) async fn bytes_under(dir: &Path) -> u64 {
	let mut total = 0_u64;
	let mut folders = vec![dir.to_path_buf()];
	while let Some(folder) = folders.pop() {
		let Ok(mut entries) = tokio::fs::read_dir(&folder).await else {
			continue;
		};
		while let Ok(Some(entry)) = entries.next_entry().await {
			// `DirEntry::metadata` does not follow a link.
			let Ok(meta) = entry.metadata().await else {
				continue;
			};
			if meta.is_dir() {
				folders.push(entry.path());
			} else {
				total = total.saturating_add(meta.len());
			}
		}
	}
	total
}

/// When the newest `*.cache.log` in `status` was written.
async fn newest_cache_warning(status: &Path) -> Option<i64> {
	let mut entries = tokio::fs::read_dir(status).await.ok()?;
	let mut newest: Option<i64> = None;
	while let Ok(Some(entry)) = entries.next_entry().await {
		if !entry.file_name().to_string_lossy().ends_with(".cache.log") {
			continue;
		}
		let Some(at) = entry.metadata().await.ok().and_then(|meta| meta.modified().ok()) else {
			continue;
		};
		let Ok(since) = at.duration_since(std::time::UNIX_EPOCH) else {
			continue;
		};
		let at = i64::try_from(since.as_secs()).unwrap_or(i64::MAX);
		newest = Some(newest.map_or(at, |seen| seen.max(at)));
	}
	newest
}

/// The drive's bytes free to an unprivileged writer, and its size.
fn drive(path: &Path) -> Option<(u64, u64)> {
	let stat = rustix::fs::statvfs(path).ok()?;
	let block = stat.f_frsize;
	Some((stat.f_bavail.saturating_mul(block), stat.f_blocks.saturating_mul(block)))
}

/// What the worker has made of the weeks home holds.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Renders {
	/// Weeks home holds.
	pub held: usize,
	/// Of those, weeks whose video the owner let go from My data (CW8): never
	/// waiting, and never rendered again.
	pub let_go: usize,
	/// Of those, weeks with no video yet, and not let go.
	pub waiting: usize,
	/// Of those, weeks whose last attempt failed.
	pub failed: usize,
	/// Unix seconds when home took the week that has waited longest, if any
	/// waits.
	pub oldest_waiting_taken: Option<i64>,
	/// Unix seconds when the worker last started a pass, if it ever has.
	pub worker_last_pass: Option<i64>,
}

/// Reads what the worker has made of each week home holds.
///
/// # Errors
/// A storage failure reading `call_week`. A file that cannot be read counts
/// as missing: a video that cannot be seen is a week still waiting.
pub async fn observe(db: &SqlitePool, renders: &Path, status: &Path) -> Result<Renders, sqlx::Error> {
	let (weeks, let_go) = {
		let mut conn = db.acquire().await?;
		(call_week_repo::taken(&mut conn).await?, call_week_repo::video::let_go(&mut conn).await?)
	};
	let mut seen = Renders {
		held: weeks.len(),
		worker_last_pass: last_pass(status).await,
		..Renders::default()
	};
	for week in &weeks {
		if let_go.contains(&week.from) {
			seen.let_go += 1;
			continue;
		}
		// `week_from` is a day by the table's own check, so it is one plain
		// path segment.
		if exists(&renders.join(&week.from).join("call.mp4")).await {
			continue;
		}
		seen.waiting += 1;
		let mut failed_log = week.from.clone();
		failed_log.push_str(".failed.log");
		if exists(&status.join(failed_log)).await {
			seen.failed += 1;
		}
		if let Ok(taken) = DateTime::parse_from_rfc3339(&week.taken_at) {
			let taken = taken.timestamp();
			seen.oldest_waiting_taken = Some(seen.oldest_waiting_taken.map_or(taken, |oldest| oldest.min(taken)));
		}
	}
	Ok(seen)
}

async fn exists(path: &Path) -> bool {
	tokio::fs::try_exists(path).await.unwrap_or(false)
}

/// The worker's `last-pass`: Unix seconds on one line, or `None`.
async fn last_pass(status: &Path) -> Option<i64> {
	let text = tokio::fs::read_to_string(status.join("last-pass")).await.ok()?;
	text.trim().parse().ok()
}

#[cfg(test)]
mod tests {
	use super::*;
	use call_week_repo::Week;
	use sqlx::sqlite::SqlitePoolOptions;

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		crate::schema::MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	async fn hold(pool: &SqlitePool, from: &str, to: &str, taken_at: &str) {
		let mut conn = pool.acquire().await.unwrap();
		let week = Week { from, to, body: b"{}", taken_at };
		call_week_repo::keep(&mut conn, &week).await.unwrap();
	}

	#[tokio::test]
	async fn the_drive_reads_as_videos_work_cache_warnings_and_free_space() {
		let dir = tempfile::tempdir().unwrap();
		let (renders, work, status) = (dir.path().join("call"), dir.path().join("call-work"), dir.path().join("call-worker"));
		assert_eq!(
			disk(&renders, &work, &status).await,
			Disk {
				drive: disk(&renders, &work, &status).await.drive,
				..Disk::default()
			},
			"nothing rendered, nothing waiting, no warning"
		);

		std::fs::create_dir_all(renders.join("2026-10-30")).unwrap();
		std::fs::write(renders.join("2026-10-30").join("call.mp4"), [0_u8; 10]).unwrap();
		std::fs::write(renders.join("2026-10-30").join("01-open.wav"), [0_u8; 5]).unwrap();
		std::fs::create_dir_all(work.join("2026-11-06").join("cache").join("steps")).unwrap();
		std::fs::write(work.join("2026-11-06").join("cache").join("steps").join("a.mkv"), [0_u8; 20]).unwrap();
		std::fs::create_dir_all(&status).unwrap();
		std::fs::write(status.join("last-pass"), b"1").unwrap();
		std::fs::write(status.join("2026-11-06.cache.log"), b"cache warning: not cached").unwrap();

		let seen = disk(&renders, &work, &status).await;
		assert_eq!((seen.videos_bytes, seen.work_bytes), (15, 20));
		assert!(seen.cache_warning_at.is_some_and(|at| at > 1_700_000_000));
		let (free, size) = seen.drive.unwrap();
		assert!(size > 0 && free <= size);
	}

	#[tokio::test]
	async fn each_week_held_reads_as_rendered_waiting_or_failed() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let (renders, status) = (dir.path().join("call"), dir.path().join("call-worker"));

		let nothing = observe(&pool, &renders, &status).await.unwrap();
		assert_eq!(nothing, Renders::default(), "no week held, no worker yet");

		hold(&pool, "2026-10-23", "2026-10-29", "2026-10-30T08:00:00.000Z").await;
		hold(&pool, "2026-10-30", "2026-11-05", "2026-11-06T08:00:00.000Z").await;
		hold(&pool, "2026-11-06", "2026-11-12", "2026-11-13T08:00:00.000Z").await;
		std::fs::create_dir_all(renders.join("2026-10-23")).unwrap();
		std::fs::write(renders.join("2026-10-23").join("call.mp4"), b"v").unwrap();
		// A folder without its video is not a video.
		std::fs::create_dir_all(renders.join("2026-11-06")).unwrap();
		std::fs::create_dir_all(&status).unwrap();
		std::fs::write(status.join("2026-10-30.failed.log"), b"the recorder exited 1\n").unwrap();
		std::fs::write(status.join("last-pass"), b"1794000000\n").unwrap();

		let seen = observe(&pool, &renders, &status).await.unwrap();
		assert_eq!(
			seen,
			Renders {
				held: 3,
				let_go: 0,
				waiting: 2,
				failed: 1,
				oldest_waiting_taken: Some(DateTime::parse_from_rfc3339("2026-11-06T08:00:00.000Z").unwrap().timestamp()),
				worker_last_pass: Some(1_794_000_000),
			}
		);

		std::fs::write(renders.join("2026-11-06").join("call.mp4"), b"v").unwrap();
		std::fs::create_dir_all(renders.join("2026-10-30")).unwrap();
		std::fs::write(renders.join("2026-10-30").join("call.mp4"), b"v").unwrap();
		std::fs::write(status.join("last-pass"), b"not a time").unwrap();
		let done = observe(&pool, &renders, &status).await.unwrap();
		assert_eq!((done.waiting, done.failed, done.oldest_waiting_taken), (0, 0, None), "a video counts whatever log is left");
		assert_eq!(done.worker_last_pass, None, "a stamp that is not a time reads as none");

		// A week whose video was let go is counted apart, never as waiting.
		std::fs::remove_file(renders.join("2026-10-23").join("call.mp4")).unwrap();
		let mut conn = pool.acquire().await.unwrap();
		call_week_repo::video::record_link(&mut conn, "2026-10-23", "dQw4w9WgXcQ", "2026-11-07T12:00:00.000Z")
			.await
			.unwrap();
		let receipt = call_week_repo::video::Receipt {
			from: "2026-10-23".to_owned(),
			step: call_week_repo::video::Step::Video,
			youtube_id: "dQw4w9WgXcQ".to_owned(),
			size: 1,
			deleted_at: "2026-11-07T12:00:00.000Z".to_owned(),
		};
		call_week_repo::video::write_receipt(&mut conn, &receipt).await.unwrap();
		drop(conn);
		let after = observe(&pool, &renders, &status).await.unwrap();
		assert_eq!((after.held, after.let_go, after.waiting, after.oldest_waiting_taken), (3, 1, 0, None));
	}
}
