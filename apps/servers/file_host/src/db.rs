//! How every process opens `file_host`'s `SQLite` database.
//!
//! The server (`main.rs`) and the operator commands under `src/bin/`
//! (`import-curriculum`, `import-leetype-rounds`, `record-leetype-runs`,
//! `dump-leetype-snapshot`) all open the same file, so the file's policy has
//! one home: [`connect_options`].

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;
use std::str::FromStr;
use std::time::Duration;

/// The size, in bytes, `SQLite` cuts the `-wal` file back to.
///
/// Without a limit, a checkpoint resets the WAL but never shrinks it, so the
/// file stays at its largest size ever: one bulk transaction on a throwaway
/// database left an 808 MB `-wal` behind for good. With one, the first commit
/// after a checkpoint has backfilled the whole log truncates the file to this
/// size. A transaction bigger than the limit still grows the file while it
/// runs; the limit bounds what is left on disk afterwards, not the peak.
///
/// Why 64 MiB: ordinary traffic stays far below it, so it never truncates a
/// file the next writes would only grow again. The auto-checkpoint runs every
/// 1000 pages, about 4 MB at the default 4 KiB page, and home's bulkiest
/// single writes are a voice note (about 240 KB a minute, so an hour is about
/// 14 MB) and an ESPN body (up to about 560 KB). 64 MiB holds several of
/// those between checkpoints.
pub const WAL_SIZE_LIMIT_BYTES: i64 = 64 * 1024 * 1024;

/// The options every opener of `file_host`'s database uses.
///
/// Callers add only what is theirs to decide: the server adds
/// `create_if_missing(true)`; the operator commands leave it off, so a
/// mistyped path fails instead of importing into a fresh empty file.
///
/// # Errors
///
/// When `database_url` is not a `SQLite` URL `sqlx` can parse.
pub fn connect_options(database_url: &str) -> Result<SqliteConnectOptions, sqlx::Error> {
	Ok(
		SqliteConnectOptions::from_str(database_url)?
			.journal_mode(SqliteJournalMode::Wal) // Allows concurrent reads/writes
			.synchronous(SqliteSynchronous::Normal) // Best performance for WAL mode
			.busy_timeout(Duration::from_secs(5)) // Prevents instant crashes on locks
			.pragma("journal_size_limit", WAL_SIZE_LIMIT_BYTES.to_string()),
	)
}

/// A one-connection pool on [`connect_options`], as the operator commands open
/// the database.
///
/// # Errors
///
/// When `database_url` does not parse or the database cannot be opened.
pub async fn connect_single(database_url: &str) -> Result<SqlitePool, sqlx::Error> {
	SqlitePoolOptions::new().max_connections(1).connect_with(connect_options(database_url)?).await
}

#[cfg(test)]
mod tests {
	use super::{connect_options, WAL_SIZE_LIMIT_BYTES};
	use sqlx::sqlite::SqlitePoolOptions;
	use std::path::Path;

	const BLOB_BYTES: i64 = 1024 * 1024;

	fn wal_bytes(db: &Path) -> u64 {
		let mut wal = db.as_os_str().to_owned();
		wal.push("-wal");
		std::fs::metadata(wal).unwrap().len()
	}

	/// A burst past the limit, a PASSIVE checkpoint, then one small write.
	/// PASSIVE, as the auto-checkpoint runs, not TRUNCATE: TRUNCATE empties
	/// the file whatever the limit, so this would pass without the pragma.
	#[tokio::test]
	async fn the_wal_file_falls_back_under_the_limit_after_a_checkpoint() {
		let dir = tempfile::tempdir().unwrap();
		let db = dir.path().join("file_host.db");
		let options = connect_options(&("sqlite://".to_owned() + &db.to_string_lossy())).unwrap();
		// One connection, so no other connection's read transaction can hold
		// the checkpoint back from backfilling the whole log.
		let pool = SqlitePoolOptions::new().max_connections(1).connect_with(options.create_if_missing(true)).await.unwrap();

		sqlx::query("CREATE TABLE burst (body BLOB NOT NULL)").execute(&pool).await.unwrap();
		let mut tx = pool.begin().await.unwrap();
		for _ in 0..=WAL_SIZE_LIMIT_BYTES / BLOB_BYTES {
			sqlx::query("INSERT INTO burst (body) VALUES (randomblob(?))")
				.bind(BLOB_BYTES)
				.execute(&mut *tx)
				.await
				.unwrap();
		}
		tx.commit().await.unwrap();
		let limit = u64::try_from(WAL_SIZE_LIMIT_BYTES).unwrap();
		let after_burst = wal_bytes(&db);
		assert!(after_burst > limit, "the burst grew the WAL past the limit ({after_burst} bytes)");

		let (busy, log, backfilled): (i64, i64, i64) = sqlx::query_as("PRAGMA wal_checkpoint(PASSIVE)").fetch_one(&pool).await.unwrap();
		assert_eq!((busy, log), (0, backfilled), "the checkpoint backfilled the whole log");
		sqlx::query("INSERT INTO burst (body) VALUES (x'00')").execute(&pool).await.unwrap();
		let after_reset = wal_bytes(&db);
		assert!(after_reset <= limit, "the WAL fell back under the limit ({after_reset} bytes)");
	}
}
