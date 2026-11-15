//! Home's record of the weekly call's weeks (paulgsc/some-ui#2635, ST36):
//! `call_week`, each week once by its first day, append-only, and
//! `call_week_revision`, each later revision of a week home took while no
//! part of it had a video (paulgsc/server#879). The newest revision of a week
//! is the one home serves. The rules are
//! `20261106000100_create_call_week.up.sql`'s and
//! `20261115000100_create_call_week_revision.up.sql`'s.

pub mod video;

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

/// What [`revise`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Revised {
	/// The bytes differ from the newest revision home keeps: stored as this
	/// revision (2 for the first re-stamp).
	Taken(i64),
	/// The bytes are the newest revision's: nothing is written.
	Same,
	/// Home holds no week of that first day: nothing is written ([`keep`]
	/// takes a new week).
	NotHeld,
}

/// A week home holds, at its newest revision, as its `call` index names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldWeek {
	pub from: String,
	/// Its last day, as its newest revision says.
	pub to: String,
	/// Lowercase hex SHA-256 of the newest revision's bytes.
	pub sha256: String,
	/// Its newest revision: 1 for the week as first taken.
	pub revision: i64,
}

/// When home took a week it holds: `call_weeks::render` reads how long a
/// week has waited for its video from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TakenWeek {
	pub from: String,
	/// ISO-8601 UTC, as [`Week::taken_at`] stored it.
	pub taken_at: String,
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

/// Stores `week` as the next revision of the week home holds of its first
/// day, unless its bytes are the newest revision's.
///
/// Only while no part of the week has a video: the table refuses a week a
/// part of which has a link or a receipt, and the caller (`call_weeks`'s
/// pass) one whose video is on the drive.
///
/// # Errors
/// A storage failure, or a revision the table refuses (a part of the week
/// has a link or a receipt, or another revision was stored meanwhile).
pub async fn revise(conn: &mut SqliteConnection, week: &Week<'_>) -> Result<Revised, sqlx::Error> {
	let Some(newest) = held_one(conn, week.from).await? else {
		return Ok(Revised::NotHeld);
	};
	let sha256 = sha256_hex(week.body);
	if newest.sha256 == sha256 {
		return Ok(Revised::Same);
	}
	let revision = newest.revision + 1;
	let size = i64::try_from(week.body.len()).unwrap_or(i64::MAX);
	sqlx::query!(
		"INSERT INTO call_week_revision (week_from, revision, week_to, sha256, size, body, taken_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
		week.from,
		revision,
		week.to,
		sha256,
		size,
		week.body,
		week.taken_at,
	)
	.execute(&mut *conn)
	.await?;
	Ok(Revised::Taken(revision))
}

/// Whether `error` is the table refusing a revision of a week a part of
/// which has a link or a receipt: the week is frozen, nothing went wrong.
#[must_use]
pub fn is_frozen(error: &sqlx::Error) -> bool {
	error.as_database_error().is_some_and(|error| error.message().contains(FROZEN))
}

/// The words `call_week_revision_in_order` refuses a frozen week with.
const FROZEN: &str = "a part of the week has a video, so the week is frozen";

/// The week home holds of first day `from`, at its newest revision.
async fn held_one(conn: &mut SqliteConnection, from: &str) -> Result<Option<HeldWeek>, sqlx::Error> {
	Ok(held(conn).await?.into_iter().find(|week| week.from == from))
}

/// Every week home holds, newest first, each at its newest revision, as its
/// `call` index lists them.
///
/// # Errors
/// A storage failure.
pub async fn held(conn: &mut SqliteConnection) -> Result<Vec<HeldWeek>, sqlx::Error> {
	sqlx::query_as!(
		HeldWeek,
		r#"SELECT week.week_from AS "from!",
		          COALESCE(newest.week_to, week.week_to) AS "to!: String",
		          COALESCE(newest.sha256, week.sha256) AS "sha256!: String",
		          COALESCE(newest.revision, 1) AS "revision!: i64"
		   FROM call_week AS week
		   LEFT JOIN call_week_revision AS newest
		     ON newest.week_from = week.week_from
		    AND newest.revision = (SELECT MAX(revision) FROM call_week_revision WHERE week_from = week.week_from)
		   ORDER BY week.week_from DESC"#
	)
	.fetch_all(&mut *conn)
	.await
}

/// Every week home holds and when it took each, oldest week first.
///
/// # Errors
/// A storage failure.
pub async fn taken(conn: &mut SqliteConnection) -> Result<Vec<TakenWeek>, sqlx::Error> {
	sqlx::query_as!(TakenWeek, r#"SELECT week_from AS "from!", taken_at AS "taken_at!" FROM call_week ORDER BY week_from"#)
		.fetch_all(&mut *conn)
		.await
}

/// How many weeks and revisions home holds, and when it last took either;
/// `None` when it holds no week. Both tables only grow, so the count alone
/// tells whether anything arrived since it was last read.
///
/// # Errors
/// A storage failure.
pub async fn as_of(conn: &mut SqliteConnection) -> Result<Option<(i64, String)>, sqlx::Error> {
	let row = sqlx::query!(
		r#"SELECT COUNT(*) AS "count!: i64", MAX(taken_at) AS "newest: String"
		   FROM (SELECT taken_at FROM call_week UNION ALL SELECT taken_at FROM call_week_revision)"#
	)
	.fetch_one(&mut *conn)
	.await?;
	Ok(row.newest.map(|newest| (row.count, newest)))
}

/// The bytes of the newest revision home holds of the week starting `from`,
/// or `None`.
///
/// # Errors
/// A storage failure.
pub async fn body(conn: &mut SqliteConnection, from: &str) -> Result<Option<Vec<u8>>, sqlx::Error> {
	let newest = sqlx::query_scalar!("SELECT body FROM call_week_revision WHERE week_from = ? ORDER BY revision DESC LIMIT 1", from)
		.fetch_optional(&mut *conn)
		.await?;
	if newest.is_some() {
		return Ok(newest);
	}
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
		let weeks = taken(&mut conn).await.unwrap();
		let taken: Vec<(&str, &str)> = weeks.iter().map(|week| (week.from.as_str(), week.taken_at.as_str())).collect();
		assert_eq!(taken, [("2026-10-30", at), ("2026-11-06", later)], "oldest first, each as first taken");
	}

	/// A re-stamp of a week with no video is a new revision, the newest of
	/// which home serves; the first revision stays as it was, and the bytes
	/// home already has are no revision (paulgsc/server#879).
	#[tokio::test]
	async fn a_restamp_is_appended_as_the_next_revision_and_served() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		let (v4, v5) = (br#"{"template":4}"#, br#"{"template":5}"#);
		let at = "2026-11-06T08:00:00.000Z";
		assert_eq!(revise(&mut conn, &week("2026-11-06", "2026-11-12", v5, at)).await.unwrap(), Revised::NotHeld);
		keep(&mut conn, &week("2026-11-06", "2026-11-12", v4, at)).await.unwrap();
		assert_eq!(revise(&mut conn, &week("2026-11-06", "2026-11-12", v4, at)).await.unwrap(), Revised::Same);
		assert_eq!(as_of(&mut conn).await.unwrap(), Some((1, at.to_owned())));

		let later = "2026-11-14T23:00:00.000Z";
		assert_eq!(revise(&mut conn, &week("2026-11-06", "2026-11-12", v5, later)).await.unwrap(), Revised::Taken(2));
		assert_eq!(revise(&mut conn, &week("2026-11-06", "2026-11-12", v5, later)).await.unwrap(), Revised::Same);
		assert_eq!(body(&mut conn, "2026-11-06").await.unwrap().as_deref(), Some(&v5[..]), "the newest is served");
		let held = held(&mut conn).await.unwrap();
		assert_eq!((held[0].sha256.as_str(), held[0].revision), (sha256_hex(v5).as_str(), 2));
		assert_eq!(as_of(&mut conn).await.unwrap(), Some((2, later.to_owned())), "a revision moves the index");
		let first: Vec<u8> = sqlx::query_scalar("SELECT body FROM call_week").fetch_one(&mut *conn).await.unwrap();
		assert_eq!(first, v4, "the first is kept as it was");

		assert_eq!(
			revise(&mut conn, &week("2026-11-06", "2026-11-12", v4, later)).await.unwrap(),
			Revised::Taken(3),
			"back again is a revision too"
		);
		assert_eq!(taken(&mut conn).await.unwrap()[0].taken_at, at, "a week has waited since it was first taken");
	}

	/// Once a part of the week has a link or a receipt, the table refuses a
	/// revision: the week is frozen at the one its video was made from.
	#[tokio::test]
	async fn a_week_with_a_video_takes_no_revision_and_none_is_rewritten() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		let at = "2026-11-06T08:00:00.000Z";
		keep(&mut conn, &week("2026-11-06", "2026-11-12", b"{}", at)).await.unwrap();
		assert_eq!(revise(&mut conn, &week("2026-11-06", "2026-11-12", b"[]", at)).await.unwrap(), Revised::Taken(2));
		let skipped = sqlx::query("INSERT INTO call_week_revision VALUES ('2026-11-06', 4, '2026-11-12', ?, 1, x'00', ?)")
			.bind("0".repeat(64))
			.bind(at)
			.execute(&mut *conn)
			.await;
		assert!(skipped.is_err(), "a revision is the next one");
		assert!(sqlx::query("UPDATE call_week_revision SET body = x'00'").execute(&mut *conn).await.is_err());
		assert!(sqlx::query("DELETE FROM call_week_revision").execute(&mut *conn).await.is_err());

		video::record_link(&mut conn, "2026-11-06-p2", "bbbbbbbbbbb", at).await.unwrap();
		let frozen = revise(&mut conn, &week("2026-11-06", "2026-11-12", b"[1]", at)).await;
		assert!(frozen.as_ref().is_err_and(is_frozen), "a part with a link freezes the week: {frozen:?}");
		assert_eq!(body(&mut conn, "2026-11-06").await.unwrap().as_deref(), Some(&b"[]"[..]));
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
