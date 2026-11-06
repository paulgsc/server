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
//! It reads file names and one number, never a week's words or a log's.
//! Cost: one read of `call_week`'s days and up to two `stat`s per week held,
//! each minute; the table grows by a week a week.

use chrono::DateTime;
use sqlx::SqlitePool;
use std::path::Path;

/// Where the worker keeps each week's video, as this server sees it.
pub const RENDERS_FOLDER: &str = "/app/dbs/renders/call";

/// Where the worker says how its passes went, as this server sees it.
pub const STATUS_FOLDER: &str = "/app/dbs/renders/call-worker";

/// What the worker has made of the weeks home holds.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Renders {
	/// Weeks home holds.
	pub held: usize,
	/// Of those, weeks with no video yet.
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
	let weeks = {
		let mut conn = db.acquire().await?;
		call_week_repo::taken(&mut conn).await?
	};
	let mut seen = Renders {
		held: weeks.len(),
		worker_last_pass: last_pass(status).await,
		..Renders::default()
	};
	for week in &weeks {
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
	}
}
