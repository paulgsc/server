//! Home's record of the clip frames it keeps (paulgsc/some-ui#2670's clips,
//! the owner's decisions of 2026-11-11): `clip_frame`, each frame once by
//! (clip, side, file), append-only. The rules are
//! `20261111000100_create_clip_frame.up.sql`'s.
//!
//! The PNG itself is on the drive (`file_host`'s `clips`); a row is home's
//! word that it holds those bytes and checked them.

use sha2::{Digest, Sha256};
use sqlx::SqliteConnection;

/// How a frame came to home.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Made {
	/// Home rendered it, on its render lane.
	Walk,
	/// Fetched from paulgsc/testing and held to a pinned SHA-256.
	Legacy,
}

impl Made {
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Walk => "walk",
			Self::Legacy => "legacy",
		}
	}
}

/// A frame to keep: its name, and the bytes home checked.
#[derive(Debug, Clone, Copy)]
pub struct Frame<'a> {
	pub clip_id: &'a str,
	/// `before` or `after`.
	pub side: &'a str,
	/// One path segment, `<name>.png`.
	pub file: &'a str,
	pub made: Made,
	pub bytes: &'a [u8],
	/// ISO-8601 UTC, when home kept it.
	pub taken_at: &'a str,
}

/// What [`keep`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
	/// New to home, and recorded.
	Added,
	/// Home already holds these very bytes under this name: nothing written.
	Held,
	/// Home holds other bytes under this name: nothing written, and the
	/// caller says so as an error. A held frame is never replaced.
	Differs,
}

/// A frame home holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldFrame {
	pub clip_id: String,
	pub side: String,
	pub file: String,
	/// Lowercase hex SHA-256 of the bytes home holds.
	pub sha256: String,
	pub size: i64,
}

/// Lowercase hex SHA-256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
	hex::encode(Sha256::digest(bytes))
}

/// Records `frame` unless home already holds one of its name.
///
/// # Errors
/// A storage failure, or a frame the table's checks refuse (a name that is
/// not plain, a side that is not `before` or `after`, an empty frame or one
/// past 1 MiB).
pub async fn keep(conn: &mut SqliteConnection, frame: &Frame<'_>) -> Result<Kept, sqlx::Error> {
	let sha256 = sha256_hex(frame.bytes);
	let size = i64::try_from(frame.bytes.len()).unwrap_or(i64::MAX);
	let made = frame.made.as_str();
	let done = sqlx::query!(
		"INSERT INTO clip_frame (clip_id, side, file, made, sha256, size, taken_at) VALUES (?, ?, ?, ?, ?, ?, ?)
		 ON CONFLICT (clip_id, side, file) DO NOTHING",
		frame.clip_id,
		frame.side,
		frame.file,
		made,
		sha256,
		size,
		frame.taken_at,
	)
	.execute(&mut *conn)
	.await?;
	if done.rows_affected() == 1 {
		return Ok(Kept::Added);
	}
	let held = one(conn, frame.clip_id, frame.side, frame.file).await?;
	Ok(if held.is_some_and(|held| held.sha256 == sha256) { Kept::Held } else { Kept::Differs })
}

/// Every frame home holds, by clip, side and file.
///
/// # Errors
/// A storage failure.
pub async fn held(conn: &mut SqliteConnection) -> Result<Vec<HeldFrame>, sqlx::Error> {
	sqlx::query_as!(
		HeldFrame,
		r#"SELECT clip_id AS "clip_id!", side AS "side!", file AS "file!", sha256 AS "sha256!", size AS "size!: i64"
		   FROM clip_frame ORDER BY clip_id, side, file"#
	)
	.fetch_all(&mut *conn)
	.await
}

/// The frame home holds under this name, or `None`.
///
/// # Errors
/// A storage failure.
pub async fn one(conn: &mut SqliteConnection, clip_id: &str, side: &str, file: &str) -> Result<Option<HeldFrame>, sqlx::Error> {
	sqlx::query_as!(
		HeldFrame,
		r#"SELECT clip_id AS "clip_id!", side AS "side!", file AS "file!", sha256 AS "sha256!", size AS "size!: i64"
		   FROM clip_frame WHERE clip_id = ? AND side = ? AND file = ?"#,
		clip_id,
		side,
		file,
	)
	.fetch_optional(&mut *conn)
	.await
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

	fn frame<'a>(file: &'a str, bytes: &'a [u8]) -> Frame<'a> {
		Frame {
			clip_id: "studio-release-promo",
			side: "after",
			file,
			made: Made::Legacy,
			bytes,
			taken_at: "2026-11-11T08:00:00.000Z",
		}
	}

	#[tokio::test]
	async fn a_frame_is_kept_once_as_taken() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(keep(&mut conn, &frame("01-templates.png", b"first")).await.unwrap(), Kept::Added);
		assert_eq!(
			keep(&mut conn, &frame("01-templates.png", b"first")).await.unwrap(),
			Kept::Held,
			"the same bytes again add nothing"
		);
		assert_eq!(
			keep(&mut conn, &frame("01-templates.png", b"other")).await.unwrap(),
			Kept::Differs,
			"other bytes under a held name are refused"
		);
		assert_eq!(keep(&mut conn, &frame("02-release-promo.png", b"second")).await.unwrap(), Kept::Added);

		let all = held(&mut conn).await.unwrap();
		let files: Vec<&str> = all.iter().map(|frame| frame.file.as_str()).collect();
		assert_eq!(files, ["01-templates.png", "02-release-promo.png"]);
		assert_eq!(all[0].sha256, sha256_hex(b"first"), "the first bytes stand");
		assert_eq!(all[0].size, 5);
		let found = one(&mut conn, "studio-release-promo", "after", "02-release-promo.png").await.unwrap();
		assert_eq!(found.map(|frame| frame.sha256), Some(sha256_hex(b"second")));
		assert_eq!(one(&mut conn, "studio-release-promo", "before", "02-release-promo.png").await.unwrap(), None);
	}

	#[tokio::test]
	async fn what_is_not_a_plain_frame_is_refused() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		for bad in [
			Frame {
				side: "middle",
				..frame("01.png", b"x")
			},
			Frame {
				clip_id: "../up",
				..frame("01.png", b"x")
			},
			frame("../01.png", b"x"),
			frame("01.jpg", b"x"),
			frame("01.png", b""),
		] {
			assert!(keep(&mut conn, &bad).await.is_err(), "{bad:?} is refused");
		}
		let past = vec![0_u8; 1_048_577];
		assert!(keep(&mut conn, &frame("01.png", &past)).await.is_err(), "past 1 MiB is refused");
		assert!(held(&mut conn).await.unwrap().is_empty());
	}

	#[tokio::test]
	async fn no_kept_frame_is_rewritten_or_deleted() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		keep(&mut conn, &frame("01.png", b"x")).await.unwrap();
		let update = sqlx::query("UPDATE clip_frame SET sha256 = ''").execute(&mut *conn).await;
		assert!(update.is_err(), "UPDATE is refused");
		let delete = sqlx::query("DELETE FROM clip_frame").execute(&mut *conn).await;
		assert!(delete.is_err(), "DELETE is refused");
		assert_eq!(held(&mut conn).await.unwrap().len(), 1);
	}
}
