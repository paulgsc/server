//! Home's record of the weekly call's weeks (paulgsc/some-ui#2635, ST36):
//! `call_week`, each week once by its first day, append-only. The rules are
//! `20261106000100_create_call_week.up.sql`'s.

use sha2::{Digest, Sha256};
use sqlx::SqliteConnection;

/// A week as the repository served it.
#[derive(Debug, Clone, Copy)]
pub struct Week<'a> {
	/// Its first day, `YYYY-MM-DD`: its file's name.
	pub from: &'a str,
	/// Its last day, both included.
	pub to: &'a str,
	/// The file's bytes, exactly as served.
	pub body: &'a [u8],
	/// ISO-8601 UTC, when home took it.
	pub taken_at: &'a str,
}

/// What [`keep`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
	/// The week is new to home, and stored.
	Added,
	/// Home already holds a week of that first day: nothing is written.
	Held,
}

/// A week home holds, as its `call` index names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldWeek {
	pub from: String,
	pub to: String,
	/// Lowercase hex SHA-256 of the bytes home holds.
	pub sha256: String,
}

/// Lowercase hex SHA-256 of `bytes`.
fn sha256_hex(bytes: &[u8]) -> String {
	hex::encode(Sha256::digest(bytes))
}

/// Stores `week` unless home already holds a week of its first day.
///
/// # Errors
/// A storage failure, or a week the table's checks refuse (a day that is not
/// ten characters, or a last day before its first).
pub async fn keep(conn: &mut SqliteConnection, week: &Week<'_>) -> Result<Kept, sqlx::Error> {
	let sha256 = sha256_hex(week.body);
	let size = i64::try_from(week.body.len()).unwrap_or(i64::MAX);
	let done = sqlx::query!(
		"INSERT INTO call_week (week_from, week_to, sha256, size, body, taken_at) VALUES (?, ?, ?, ?, ?, ?)
		 ON CONFLICT (week_from) DO NOTHING",
		week.from,
		week.to,
		sha256,
		size,
		week.body,
		week.taken_at,
	)
	.execute(&mut *conn)
	.await?;
	Ok(if done.rows_affected() == 1 { Kept::Added } else { Kept::Held })
}

/// Every week home holds, newest first, as its `call` index lists them.
///
/// # Errors
/// A storage failure.
pub async fn held(conn: &mut SqliteConnection) -> Result<Vec<HeldWeek>, sqlx::Error> {
	sqlx::query_as!(
		HeldWeek,
		r#"SELECT week_from AS "from!", week_to AS "to!", sha256 AS "sha256!" FROM call_week ORDER BY week_from DESC"#
	)
	.fetch_all(&mut *conn)
	.await
}

/// How many weeks home holds, and when it last took one; `None` when it
/// holds none. The table only grows, so the count alone tells whether
/// anything arrived since it was last read.
///
/// # Errors
/// A storage failure.
pub async fn as_of(conn: &mut SqliteConnection) -> Result<Option<(i64, String)>, sqlx::Error> {
	let row = sqlx::query!(r#"SELECT COUNT(*) AS "count!: i64", MAX(taken_at) AS "newest: String" FROM call_week"#)
		.fetch_one(&mut *conn)
		.await?;
	Ok(row.newest.map(|newest| (row.count, newest)))
}

/// The bytes home holds for the week starting `from`, or `None`.
///
/// # Errors
/// A storage failure.
pub async fn body(conn: &mut SqliteConnection, from: &str) -> Result<Option<Vec<u8>>, sqlx::Error> {
	sqlx::query_scalar!("SELECT body FROM call_week WHERE week_from = ?", from).fetch_optional(&mut *conn).await
}

#[cfg(test)]
mod tests {
	use super::*;
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn week<'a>(from: &'a str, to: &'a str, body: &'a [u8], taken_at: &'a str) -> Week<'a> {
		Week { from, to, body, taken_at }
	}

	#[tokio::test]
	async fn a_week_is_kept_once_as_served() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(as_of(&mut conn).await.unwrap(), None);
		let first = br#"{"schema":"some-ui.call-week/1"}"#;
		let at = "2026-11-06T08:00:00.000Z";
		assert_eq!(keep(&mut conn, &week("2026-10-30", "2026-11-05", first, at)).await.unwrap(), Kept::Added);
		let edited = br#"{"schema":"some-ui.call-week/1","edited":true}"#;
		let later = "2026-11-06T09:00:00.000Z";
		assert_eq!(
			keep(&mut conn, &week("2026-10-30", "2026-11-05", edited, later)).await.unwrap(),
			Kept::Held,
			"an edit is not taken"
		);
		assert_eq!(body(&mut conn, "2026-10-30").await.unwrap().as_deref(), Some(&first[..]));
		assert_eq!(keep(&mut conn, &week("2026-11-06", "2026-11-12", edited, later)).await.unwrap(), Kept::Added);

		let listed = held(&mut conn).await.unwrap();
		let froms: Vec<&str> = listed.iter().map(|week| week.from.as_str()).collect();
		assert_eq!(froms, ["2026-11-06", "2026-10-30"], "newest first");
		assert_eq!(listed[1].sha256, sha256_hex(first));
		assert_eq!(listed[1].to, "2026-11-05");
		assert_eq!(as_of(&mut conn).await.unwrap(), Some((2, later.to_owned())));
	}

	#[tokio::test]
	async fn no_kept_week_is_rewritten_or_deleted() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		keep(&mut conn, &week("2026-10-30", "2026-11-05", b"{}", "2026-11-06T08:00:00.000Z")).await.unwrap();
		let update = sqlx::query("UPDATE call_week SET body = x'00'").execute(&mut *conn).await;
		assert!(update.is_err(), "UPDATE is refused");
		let delete = sqlx::query("DELETE FROM call_week").execute(&mut *conn).await;
		assert!(delete.is_err(), "DELETE is refused");
		assert_eq!(held(&mut conn).await.unwrap().len(), 1);
	}
}
