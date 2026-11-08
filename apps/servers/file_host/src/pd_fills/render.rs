//! Playoff Delta's fills for home's render worker, and what it has made of
//! them (paulgsc/some-ui#2647, FEED4).
//!
//! **What the worker is given.** [`export`] writes each fill home holds of a
//! voiced version ([`VOICED_VERSION`] on) to [`EPISODES_FOLDER`], as
//! `<key>/fill.json` and `<key>/season.json`, byte for byte as home keeps
//! them; the key is `<season>-<team>-<NN>` (`pd_fill_repo::HeldFill::key`).
//! A fill written for the owner's own voice (version 1) is never written:
//! the recorder refuses it, and home keeps it as the record of what was said.
//! `infra/render/render-calls.sh`'s `playoff-delta` job renders each episode
//! there with no `episode.mp4` in [`RENDERS_FOLDER`].
//!
//! **What it has made of them.** [`observe`] reads the worker's folders
//! against `pd_fill`, as `call_weeks::render` reads the call's
//! (`call_weeks::render::tally`), for the `PlayoffDeltaRender*` alerts (PK5)
//! and the CALL dashboard.
//!
//! It reads file names and one number, never a fill's words or a log's.
//! Cost: one read of `pd_fill`'s keys and up to two `stat`s per voiced fill
//! held, each minute; the table grows by a fill a week in season.

use crate::call_weeks::render::{tally, Held, Renders};
use sqlx::SqlitePool;
use std::path::Path;

/// The first format version home's speaker says: `VOICED_VERSION` in
/// paulgsc/some-ui's `@some-ui/playoff-delta`. A fill of an earlier version
/// was written for the owner's own voice.
pub const VOICED_VERSION: i64 = 2;

/// Where each voiced fill home holds is written for the render worker.
///
/// Beside the database: on the host, `${STORAGE}/playoff-delta-episodes`, mounted
/// read-only into the worker as `/episodes` (`infra/compose/call-render.yml`).
pub const EPISODES_FOLDER: &str = "/app/dbs/playoff-delta-episodes";

/// Where the worker keeps each episode's video, as this server sees it.
pub const RENDERS_FOLDER: &str = "/app/dbs/renders/playoff-delta";

/// Where the worker says how its passes went, as this server sees it.
pub const STATUS_FOLDER: &str = "/app/dbs/renders/playoff-delta-worker";

/// Where the worker keeps a waiting episode's work and step cache between
/// attempts, as this server sees it.
pub const WORK_FOLDER: &str = "/app/dbs/renders/playoff-delta-work";

/// The video's file in an episode's folder.
pub const VIDEO: &str = "episode.mp4";

/// What [`export`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Exported {
	/// Episodes written to the folder.
	pub written: usize,
	/// Episodes that could not be written.
	pub failed: usize,
}

/// Writes each voiced fill home holds that `folder` has no episode of.
///
/// Each episode is written in a temporary folder and renamed into place
/// whole, so the worker never reads half of one. An episode already written
/// is left alone: home never rewrites a kept fill, so neither does this.
///
/// # Errors
/// A storage failure.
pub async fn export(db: &SqlitePool, folder: &Path) -> Result<Exported, sqlx::Error> {
	let mut done = Exported::default();
	let held = {
		let mut conn = db.acquire().await?;
		pd_fill_repo::held(&mut conn).await?
	};
	let missing: Vec<_> = held
		.into_iter()
		.filter(|fill| fill.version >= VOICED_VERSION && !folder.join(fill.key()).exists())
		.collect();
	if missing.is_empty() {
		return Ok(done);
	}
	if tokio::fs::create_dir_all(folder).await.is_err() {
		done.failed += missing.len();
		return Ok(done);
	}
	for fill in missing {
		let bodies = {
			let mut conn = db.acquire().await?;
			pd_fill_repo::bodies(&mut conn, &fill.season, &fill.team, fill.week).await?
		};
		let Some((body, season)) = bodies else { continue };
		let key = fill.key();
		let partial = folder.join(String::from(".") + &key + ".partial");
		if write_whole(&partial, &folder.join(&key), &body, &season).await.is_ok() {
			done.written += 1;
		} else {
			done.failed += 1;
		}
	}
	Ok(done)
}

/// Writes the fill and its season into `partial`, then renames it to `into`.
async fn write_whole(partial: &Path, into: &Path, fill: &[u8], season: &[u8]) -> std::io::Result<()> {
	// What a stopped pass left is made again.
	match tokio::fs::remove_dir_all(partial).await {
		Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
		_ => {}
	}
	tokio::fs::create_dir_all(partial).await?;
	tokio::fs::write(partial.join("fill.json"), fill).await?;
	tokio::fs::write(partial.join("season.json"), season).await?;
	tokio::fs::rename(partial, into).await
}

/// What the worker has made of Playoff Delta's fills.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Seen {
	/// Every fill home holds, of any version.
	pub fills: usize,
	/// Of those, fills written for the owner's own voice: kept, never
	/// rendered.
	pub unvoiced: usize,
	/// The voiced ones, as the worker has made them.
	pub renders: Renders,
}

/// Reads what the worker has made of each voiced fill home holds.
///
/// # Errors
/// A storage failure reading `pd_fill`. A file that cannot be read counts as
/// missing: a video that cannot be seen is an episode still waiting.
pub async fn observe(db: &SqlitePool, renders: &Path, status: &Path) -> Result<Seen, sqlx::Error> {
	let held = {
		let mut conn = db.acquire().await?;
		pd_fill_repo::held(&mut conn).await?
	};
	let fills = held.len();
	let voiced: Vec<Held> = held
		.into_iter()
		.filter(|fill| fill.version >= VOICED_VERSION)
		.map(|fill| Held {
			key: fill.key(),
			taken_at: fill.taken_at,
		})
		.collect();
	Ok(Seen {
		fills,
		unvoiced: fills - voiced.len(),
		renders: tally(&voiced, &[], renders, status, VIDEO).await,
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use pd_fill_repo::Fill;
	use sqlx::sqlite::SqlitePoolOptions;

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		crate::schema::MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	async fn hold(pool: &SqlitePool, week: i64, version: i64, taken_at: &str) {
		let mut conn = pool.acquire().await.unwrap();
		let body = String::from("fill ") + &week.to_string();
		let fill = Fill {
			season: "2026",
			team: "SF",
			week,
			version,
			body: body.as_bytes(),
			season_body: b"season",
			taken_at,
		};
		pd_fill_repo::keep(&mut conn, &fill).await.unwrap();
	}

	#[tokio::test]
	async fn each_voiced_fill_is_written_for_the_worker_once_and_one_for_my_voice_never() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let folder = dir.path().join("episodes");
		hold(&pool, 8, 1, "2026-11-07T00:00:00.000Z").await;
		hold(&pool, 9, 2, "2026-11-10T20:00:00.000Z").await;

		assert_eq!(export(&pool, &folder).await.unwrap(), Exported { written: 1, failed: 0 });
		assert_eq!(std::fs::read(folder.join("2026-SF-09").join("fill.json")).unwrap(), b"fill 9");
		assert_eq!(std::fs::read(folder.join("2026-SF-09").join("season.json")).unwrap(), b"season");
		let names: Vec<_> = std::fs::read_dir(&folder).unwrap().map(|entry| entry.unwrap().file_name()).collect();
		assert_eq!(names, ["2026-SF-09"], "no version 1 fill, and no partial folder");

		std::fs::write(folder.join("2026-SF-09").join("fill.json"), b"touched").unwrap();
		std::fs::create_dir_all(folder.join(".2026-SF-10.partial")).unwrap();
		hold(&pool, 10, 2, "2026-11-17T20:00:00.000Z").await;
		assert_eq!(export(&pool, &folder).await.unwrap().written, 1, "only the new one");
		assert_eq!(
			std::fs::read(folder.join("2026-SF-09").join("fill.json")).unwrap(),
			b"touched",
			"a written episode is left alone"
		);
		assert!(folder.join("2026-SF-10").join("season.json").exists(), "a stopped write is made again");
		assert!(!folder.join(".2026-SF-10.partial").exists());
	}

	#[tokio::test]
	async fn each_voiced_fill_reads_as_rendered_waiting_or_failed_and_mine_apart() {
		let (pool, dir) = (pool().await, tempfile::tempdir().unwrap());
		let (renders, status) = (dir.path().join("playoff-delta"), dir.path().join("playoff-delta-worker"));
		hold(&pool, 0, 1, "2026-11-06T00:00:00.000Z").await;
		hold(&pool, 9, 2, "2026-11-10T20:00:00.000Z").await;
		hold(&pool, 10, 2, "2026-11-17T20:00:00.000Z").await;
		std::fs::create_dir_all(renders.join("2026-SF-09")).unwrap();
		std::fs::write(renders.join("2026-SF-09").join(VIDEO), b"v").unwrap();
		std::fs::create_dir_all(&status).unwrap();
		std::fs::write(status.join("2026-SF-10.failed.log"), b"the recorder exited 1\n").unwrap();
		std::fs::write(status.join("last-pass"), b"1794000000\n").unwrap();

		let seen = observe(&pool, &renders, &status).await.unwrap();
		assert_eq!((seen.fills, seen.unvoiced), (3, 1));
		assert_eq!(
			seen.renders,
			Renders {
				held: 2,
				let_go: 0,
				waiting: 1,
				failed: 1,
				oldest_waiting_taken: Some(chrono::DateTime::parse_from_rfc3339("2026-11-17T20:00:00.000Z").unwrap().timestamp()),
				worker_last_pass: Some(1_794_000_000),
			}
		);
	}
}
