//! What home keeps, by table, once a day (paulgsc/server#542, STORAGE1).
//!
//! `docs/storage.md` is the design. In short:
//!
//! - **Every hour, at most one sample a day.** [`spawn`]'s first check runs
//!   at once and then hourly. A check samples only when `storage_sample`
//!   holds no set for today (UTC), so a restart never samples a day twice,
//!   and a host that was down at any given hour still samples that day.
//! - **Bounded, and it yields.** [`check_once`] reads one b-tree at a time
//!   from `dbstat`, each on a connection taken from the pool and handed back,
//!   and stops between any two, or before the write, at the shutdown token
//!   or [`PASS_DEADLINE`].
//!   A stopped or failed pass writes nothing; the next hour tries again. A
//!   failure is logged and counted (`storage_sample_failing`) and fails
//!   nothing else.
//! - **The windows are read on the same hourly check** ([`storage_repo::figures`]:
//!   three sets, whatever the history's length) and exported as gauges
//!   (`metrics::storage`), since Prometheus keeps 200 hours and a window is
//!   90 days.
//! - [`refresh_footprint`] is the O(1) part, on `metrics::periodic`'s 10 s
//!   sweep: the file's size and free pages from its header, and the WAL's
//!   length from one `stat`.

use crate::AppState;
use chrono::{DateTime, SecondsFormat, Utc};
use sqlx::SqlitePool;
use std::{ffi::OsString, time::Duration};
use storage_repo::{Change, Kind, Size};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

/// How often home checks whether today is sampled.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// The most one pass may spend reading b-trees before it stops and writes
/// nothing. `docs/storage.md`, "What the daily pass costs", says why this.
pub const PASS_DEADLINE: Duration = Duration::from_secs(5 * 60);

/// How long the 10 s sweep waits for a connection before skipping a tick.
const FOOTPRINT_WAIT: Duration = Duration::from_secs(1);

/// What one check did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Checked {
	/// Today's set is stored now: this many rows.
	Sampled(usize),
	/// Today was already held: nothing read.
	Held,
	/// Shutdown came mid-pass: nothing written.
	Cancelled,
}

/// Why a check wrote nothing.
#[derive(Debug, thiserror::Error)]
pub enum PassError {
	#[error("storage: {0}")]
	Storage(#[from] sqlx::Error),
	#[error("the database's filesystem: {0}")]
	Filesystem(#[from] std::io::Error),
	#[error("ran out of time after {measured} of {of} b-trees")]
	Deadline { measured: usize, of: usize },
}

/// Spawn the hourly check, cancelled through the shared token. Unconditional:
/// every deployment keeps a database.
pub fn spawn(state: &AppState) {
	let cancel = state.core.cancel_token.clone();
	let db = state.core.shared_db.clone();
	crate::metrics::storage::record_pass_deadline(PASS_DEADLINE);

	tokio::spawn(async move {
		info!(interval_secs = CHECK_INTERVAL.as_secs(), "storage sampler started");
		let mut ticker = tokio::time::interval(CHECK_INTERVAL);
		ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		loop {
			tokio::select! {
				() = cancel.cancelled() => {
					info!("storage sampler cancelled");
					return;
				}
				_ = ticker.tick() => {
					let started = Instant::now();
					let result = check_once(&db, Utc::now(), PASS_DEADLINE, &cancel).await;
					match &result {
						Ok(Checked::Sampled(rows)) => {
							crate::metrics::storage::record_pass_duration(started.elapsed());
							info!(rows, elapsed_ms = started.elapsed().as_millis(), "home sampled what it keeps");
						}
						Ok(Checked::Held) => {}
						Ok(Checked::Cancelled) => {
							info!("storage sample stopped for shutdown; nothing written");
							return;
						}
						Err(err) => {
							if matches!(err, PassError::Deadline { .. }) {
								crate::metrics::storage::record_pass_duration(started.elapsed());
							}
							error!(error = %err, "storage sample failed; nothing written, the next check tries again");
						}
					}
					crate::metrics::storage::record_failing(result.is_err());
					refresh_figures(&db).await;
				}
			}
		}
	});
}

/// Sample today unless `storage_sample` already holds it.
///
/// `now` names the day and stamps the set. Reads one b-tree per step, each
/// on its own pooled connection, checking `cancel` and `deadline` between
/// steps and before the write, so a step is the most the pass outlasts
/// either by, and a pass that outlasts either writes nothing.
///
/// # Errors
/// [`PassError`]; nothing is written then.
pub async fn check_once(db: &SqlitePool, now: DateTime<Utc>, deadline: Duration, cancel: &CancellationToken) -> Result<Checked, PassError> {
	let day = now.date_naive().to_string();
	let sampled_at = now.to_rfc3339_opts(SecondsFormat::Millis, true);
	if storage_repo::holds_day(&mut *db.acquire().await?, &day).await? {
		return Ok(Checked::Held);
	}

	let started = Instant::now();
	let btrees = storage_repo::btrees(&mut *db.acquire().await?).await?;
	let of = btrees.len();
	let mut sizes = Vec::with_capacity(of + 3);
	for (kind, name) in btrees {
		if let Some(stop) = stop(cancel, started, deadline, sizes.len(), of) {
			return stop;
		}
		let bytes = storage_repo::btree_bytes(&mut *db.acquire().await?, &name).await?;
		sizes.push(Size { kind, name, bytes });
		tokio::task::yield_now().await;
	}

	let footprint = storage_repo::footprint(&mut *db.acquire().await?).await?;
	sizes.push(Size::whole(Kind::Database, footprint.database_bytes));
	if !footprint.file.is_empty() {
		sizes.push(Size::whole(Kind::Wal, saturating(wal_bytes(&footprint.file)?)));
		sizes.push(Size::whole(Kind::DiskFree, saturating(disk_free_bytes(&footprint.file)?)));
	}

	// The last step may have run past the deadline, or shutdown come during
	// it: then the set is not written either.
	if let Some(stop) = stop(cancel, started, deadline, of, of) {
		return stop;
	}
	match storage_repo::record(&mut *db.acquire().await?, &day, &sampled_at, &sizes).await? {
		Change::Added(rows) => Ok(Checked::Sampled(rows)),
		Change::Unchanged => Ok(Checked::Held),
	}
}

/// Whether the pass stops here, and how: at shutdown, or past its deadline.
fn stop(cancel: &CancellationToken, started: Instant, deadline: Duration, measured: usize, of: usize) -> Option<Result<Checked, PassError>> {
	if cancel.is_cancelled() {
		Some(Ok(Checked::Cancelled))
	} else if started.elapsed() >= deadline {
		Some(Err(PassError::Deadline { measured, of }))
	} else {
		None
	}
}

/// Read the newest set's figures and export them. A failure is logged and
/// leaves the gauges as they were.
pub async fn refresh_figures(db: &SqlitePool) {
	let figures = match db.acquire().await {
		Ok(mut conn) => storage_repo::figures(&mut conn).await,
		Err(err) => Err(err),
	};
	match figures {
		Ok(Some(figures)) => crate::metrics::storage::record_figures(&figures),
		Ok(None) => debug!("no storage sample held yet"),
		Err(err) => error!(error = %err, "could not read the storage sample's figures"),
	}
}

/// The 10 s sweep's sizes. A tick that waits more than a second for a
/// connection, or fails, is skipped; the gauges keep their last values.
pub async fn refresh_footprint(db: &SqlitePool) {
	let footprint = match tokio::time::timeout(FOOTPRINT_WAIT, db.acquire()).await {
		Ok(Ok(mut conn)) => storage_repo::footprint(&mut conn).await,
		Ok(Err(err)) => Err(err),
		Err(_) => {
			debug!("no connection within a second; database size not read this tick");
			return;
		}
	};
	match footprint {
		Ok(footprint) => {
			let wal = if footprint.file.is_empty() {
				None
			} else {
				match wal_bytes(&footprint.file) {
					Ok(bytes) => Some(bytes),
					Err(err) => {
						warn!(error = %err, "could not read the WAL's length");
						None
					}
				}
			};
			crate::metrics::storage::record_footprint(footprint.database_bytes, footprint.freelist_bytes, wal);
		}
		Err(err) => debug!(error = %err, "could not read the database's size"),
	}
}

/// The `-wal` file's length beside `database`, 0 when there is none (no
/// write since the last connection closed, or not in WAL mode).
fn wal_bytes(database: &str) -> Result<u64, std::io::Error> {
	let mut wal = OsString::from(database);
	wal.push("-wal");
	match std::fs::metadata(&wal) {
		Ok(metadata) => Ok(metadata.len()),
		Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(0),
		Err(err) => Err(err),
	}
}

/// The bytes free to an unprivileged writer on the filesystem holding
/// `database`: `statvfs`'s available blocks times their size.
fn disk_free_bytes(database: &str) -> Result<u64, std::io::Error> {
	let stat = rustix::fs::statvfs(database)?;
	Ok(stat.f_bavail.saturating_mul(stat.f_frsize))
}

fn saturating(bytes: u64) -> i64 {
	i64::try_from(bytes).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
	use super::{check_once, Checked, PassError};
	use chrono::{DateTime, Utc};
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;
	use std::time::Duration;
	use storage_repo::Kind;
	use tokio_util::sync::CancellationToken;

	const DEADLINE: Duration = Duration::from_secs(60);

	/// A migrated database in a file, opened as home opens it, so the sample
	/// has a WAL and a filesystem to read.
	async fn open(path: &std::path::Path) -> SqlitePool {
		let options = crate::db::connect_options(&("sqlite://".to_owned() + &path.to_string_lossy())).unwrap();
		let pool = SqlitePoolOptions::new().max_connections(2).connect_with(options.create_if_missing(true)).await.unwrap();
		crate::schema::MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn utc(at: &str) -> DateTime<Utc> {
		DateTime::parse_from_rfc3339(at).unwrap().with_timezone(&Utc)
	}

	async fn rows(pool: &SqlitePool, day: &str) -> Vec<(String, String, i64)> {
		sqlx::query_as("SELECT kind, name, bytes FROM storage_sample WHERE day = ?1 ORDER BY kind, name")
			.bind(day)
			.fetch_all(pool)
			.await
			.unwrap()
	}

	#[tokio::test]
	async fn one_set_a_day_across_a_restart() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("home.db");
		let cancel = CancellationToken::new();

		let pool = open(&path).await;
		let first = check_once(&pool, utc("2026-10-29T03:00:00Z"), DEADLINE, &cancel).await.unwrap();
		let Checked::Sampled(count) = first else { panic!("{first:?}") };
		let set = rows(&pool, "2026-10-29").await;
		assert_eq!(set.len(), count);
		for (kind, name) in [
			("database", "database"),
			("wal", "wal"),
			("disk_free", "disk_free"),
			("table", "storage_sample"),
			("table", "voice_note"),
			("table", "sqlite_schema"),
		] {
			assert!(set.iter().any(|(k, n, _)| k == kind && n == name), "{kind} {name} is in the set");
		}
		assert!(set.iter().any(|(k, _, _)| k == "index"));
		let tables: i64 = set.iter().filter(|(k, _, _)| k == "table" || k == "index").map(|(_, _, b)| b).sum();
		let database = set.iter().find(|(k, _, _)| k == "database").unwrap().2;
		assert!(tables <= database && tables > 0, "the b-trees are inside the file: {tables} of {database}");
		assert!(set.iter().find(|(k, _, _)| k == "disk_free").unwrap().2 > 0);

		// The same day again, in this process and after a restart: nothing.
		assert_eq!(check_once(&pool, utc("2026-10-29T04:00:00Z"), DEADLINE, &cancel).await.unwrap(), Checked::Held);
		pool.close().await;
		let pool = open(&path).await;
		assert_eq!(check_once(&pool, utc("2026-10-29T23:59:00Z"), DEADLINE, &cancel).await.unwrap(), Checked::Held);
		assert_eq!(rows(&pool, "2026-10-29").await.len(), count);

		// The next day is a new set, whatever hour the host was up.
		assert!(matches!(
			check_once(&pool, utc("2026-10-30T17:00:00Z"), DEADLINE, &cancel).await.unwrap(),
			Checked::Sampled(_)
		));
		let figures = storage_repo::figures(&mut pool.acquire().await.unwrap()).await.unwrap().unwrap();
		assert_eq!((figures.day.as_str(), figures.short.span_days), ("2026-10-30", 1));
		assert!(figures.sizes.iter().any(|size| size.kind == Kind::DiskFree));
	}

	#[tokio::test]
	async fn a_pass_out_of_time_or_cancelled_writes_nothing() {
		let dir = tempfile::tempdir().unwrap();
		let pool = open(&dir.path().join("home.db")).await;
		let now = utc("2026-10-29T03:00:00Z");

		let err = check_once(&pool, now, Duration::ZERO, &CancellationToken::new()).await.unwrap_err();
		assert!(matches!(err, PassError::Deadline { measured: 0, of } if of > 0), "{err}");
		let cancel = CancellationToken::new();
		cancel.cancel();
		assert_eq!(check_once(&pool, now, DEADLINE, &cancel).await.unwrap(), Checked::Cancelled);
		assert!(rows(&pool, "2026-10-29").await.is_empty());
		assert!(matches!(check_once(&pool, now, DEADLINE, &CancellationToken::new()).await.unwrap(), Checked::Sampled(_)));
	}

	/// What `/metrics` renders after a sweep, two daily samples and a
	/// refresh: the series the sweep and the figures set.
	#[test]
	fn the_gauges_reach_the_scrape() {
		let recorder = some_metrics::PrometheusBuilder::new().build_recorder();
		let handle = recorder.handle();
		metrics::with_local_recorder(&recorder, || {
			let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
			runtime.block_on(async {
				let dir = tempfile::tempdir().unwrap();
				let pool = open(&dir.path().join("home.db")).await;
				let cancel = CancellationToken::new();
				super::refresh_footprint(&pool).await;
				for at in ["2026-10-28T03:00:00Z", "2026-10-29T03:00:00Z"] {
					check_once(&pool, utc(at), DEADLINE, &cancel).await.unwrap();
				}
				super::refresh_figures(&pool).await;
			});
		});
		let scrape = handle.render();
		for series in [
			"sqlite_db_bytes ",
			"sqlite_freelist_bytes ",
			"sqlite_wal_bytes ",
			"storage_sample_timestamp_seconds ",
			"storage_sample_bytes{kind=\"table\",name=\"voice_note\"} ",
			"storage_sample_bytes{kind=\"disk_free\",name=\"disk_free\"} ",
			"storage_growth_span_days{window=\"7d\"} 1",
			"storage_growth_span_days{window=\"90d\"} 1",
			"storage_growth_bytes_per_day{window=\"7d\"} ",
			"storage_table_growth_7d_bytes{kind=\"index\",name=\"sqlite_autoindex_storage_sample_1\"} ",
			"storage_runway_days ",
		] {
			assert!(scrape.lines().any(|line| line.starts_with(series)), "{series} is scraped:\n{scrape}");
		}
	}
}
