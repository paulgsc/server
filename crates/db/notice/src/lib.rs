//! The Jobs arrival notice's two tables
//! (`20261025000300_create_jobs_notice.up.sql` records the design):
//!
//! - **`jobs_notice_device`**, the phones that asked to be told when the
//!   morning run is in. A standing request: [`register`] adds one,
//!   [`forget`] removes it (the phone stopped asking, or FCM says it is
//!   gone). A token is never logged (docs/identity.md invariant 24).
//! - **`jobs_notice_sent`**, append-only: one row per run record home
//!   noticed, so a record notifies at most once ([`record_sent`],
//!   [`was_noticed`]).
//!
//! Every query takes the subject and filters on it. The writers are
//! `file_host`'s `handlers::jobs` (`PUT` and `DELETE /api/v1/jobs/notice`) and
//! its `jobs_notice` pass, each under account deletion's hold.

use sqlx::SqliteConnection;

/// What a write did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
	/// It is stored now.
	Added,
	/// It was already there: nothing written.
	Unchanged,
	/// The subject has no account: nothing written.
	NoAccount,
}

/// What home said of one run record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Said {
	/// The picks are at home.
	Picks,
	/// The run couldn't finish.
	Trouble,
	/// A re-run that added nothing, on a morning another record's picks
	/// already spoke for: nothing sent.
	Skipped,
}

impl Said {
	/// The value `jobs_notice_sent.said` stores.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Picks => "picks",
			Self::Trouble => "trouble",
			Self::Skipped => "skipped",
		}
	}
}

/// Add `token` to the phones `subject` wants told, as of `now`.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn register(conn: &mut SqliteConnection, subject: &str, token: &str, now: &str) -> Result<Change, sqlx::Error> {
	let held = sqlx::query_scalar!("SELECT COUNT(*) FROM jobs_notice_device WHERE subject_id = ?1 AND token = ?2", subject, token)
		.fetch_one(&mut *conn)
		.await?;
	if held > 0 {
		return Ok(Change::Unchanged);
	}
	let written = sqlx::query!(
		"INSERT INTO jobs_notice_device (subject_id, token, registered_at)
		 SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM account WHERE subject_id = ?1)",
		subject,
		token,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok(if written.rows_affected() == 0 { Change::NoAccount } else { Change::Added })
}

/// Stop telling the phone `token` names; whether it was there.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn forget(conn: &mut SqliteConnection, subject: &str, token: &str) -> Result<bool, sqlx::Error> {
	let removed = sqlx::query!("DELETE FROM jobs_notice_device WHERE subject_id = ?1 AND token = ?2", subject, token)
		.execute(&mut *conn)
		.await?;
	Ok(removed.rows_affected() > 0)
}

/// The tokens of the phones `subject` wants told, oldest request first.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn devices(conn: &mut SqliteConnection, subject: &str) -> Result<Vec<String>, sqlx::Error> {
	sqlx::query_scalar!("SELECT token FROM jobs_notice_device WHERE subject_id = ?1 ORDER BY registered_at, token", subject)
		.fetch_all(&mut *conn)
		.await
}

/// Whether home already noticed the run record `run_name`.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn was_noticed(conn: &mut SqliteConnection, subject: &str, run_name: &str) -> Result<bool, sqlx::Error> {
	let held = sqlx::query_scalar!("SELECT COUNT(*) FROM jobs_notice_sent WHERE subject_id = ?1 AND run_name = ?2", subject, run_name)
		.fetch_one(&mut *conn)
		.await?;
	Ok(held > 0)
}

/// Record that home noticed `run_name`: what it said, and to how many phones.
/// A record already noticed is left as it was.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn record_sent(conn: &mut SqliteConnection, subject: &str, run_name: &str, said: Said, devices: i64, now: &str) -> Result<Change, sqlx::Error> {
	if was_noticed(conn, subject, run_name).await? {
		return Ok(Change::Unchanged);
	}
	let said = said.as_str();
	let written = sqlx::query!(
		"INSERT INTO jobs_notice_sent (subject_id, run_name, said, devices, at)
		 SELECT ?1, ?2, ?3, ?4, ?5 WHERE EXISTS (SELECT 1 FROM account WHERE subject_id = ?1)",
		subject,
		run_name,
		said,
		devices,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok(if written.rows_affected() == 0 { Change::NoAccount } else { Change::Added })
}

#[cfg(test)]
mod tests {
	use super::{devices, forget, record_sent, register, was_noticed, Change, Said};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		for subject in ["a", "b"] {
			sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?1)")
				.bind(subject)
				.execute(&pool)
				.await
				.unwrap();
		}
		pool
	}

	#[tokio::test]
	async fn a_phone_registers_once_and_is_forgotten_per_subject() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(register(&mut conn, "a", "t1", "2026-10-25T19:00:00.000Z").await.unwrap(), Change::Added);
		assert_eq!(register(&mut conn, "a", "t1", "2026-10-25T19:05:00.000Z").await.unwrap(), Change::Unchanged);
		assert_eq!(register(&mut conn, "a", "t2", "2026-10-25T19:10:00.000Z").await.unwrap(), Change::Added);
		assert_eq!(register(&mut conn, "nobody", "t3", "2026-10-25T19:10:00.000Z").await.unwrap(), Change::NoAccount);
		assert_eq!(devices(&mut conn, "a").await.unwrap(), ["t1", "t2"]);
		assert!(devices(&mut conn, "b").await.unwrap().is_empty(), "b holds none of a's");
		assert!(!forget(&mut conn, "b", "t1").await.unwrap(), "b cannot forget a's phone");
		assert!(forget(&mut conn, "a", "t1").await.unwrap());
		assert_eq!(devices(&mut conn, "a").await.unwrap(), ["t2"]);
	}

	#[tokio::test]
	async fn a_run_is_noticed_once_and_never_rewritten() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		let run = "run-jobs-20261026T145207Z.json";
		assert!(!was_noticed(&mut conn, "a", run).await.unwrap());
		assert_eq!(record_sent(&mut conn, "a", run, Said::Picks, 1, "2026-10-26T15:05:00.000Z").await.unwrap(), Change::Added);
		assert_eq!(
			record_sent(&mut conn, "a", run, Said::Trouble, 2, "2026-10-26T15:20:00.000Z").await.unwrap(),
			Change::Unchanged
		);
		assert!(was_noticed(&mut conn, "a", run).await.unwrap());
		assert!(!was_noticed(&mut conn, "b", run).await.unwrap(), "b's runs are b's");
		let update = sqlx::query("UPDATE jobs_notice_sent SET said = 'trouble'").execute(&mut *conn).await;
		assert!(update.is_err(), "the trigger refuses a rewrite");
	}

	#[tokio::test]
	async fn both_leave_with_the_account() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		register(&mut conn, "a", "t1", "now").await.unwrap();
		register(&mut conn, "b", "t1", "now").await.unwrap();
		record_sent(&mut conn, "a", "run-jobs-1.json", Said::Skipped, 0, "now").await.unwrap();
		sqlx::query("DELETE FROM account WHERE subject_id = 'a'").execute(&mut *conn).await.unwrap();
		assert!(devices(&mut conn, "a").await.unwrap().is_empty());
		assert!(!was_noticed(&mut conn, "a", "run-jobs-1.json").await.unwrap());
		assert_eq!(devices(&mut conn, "b").await.unwrap(), ["t1"], "b stays");
	}
}
