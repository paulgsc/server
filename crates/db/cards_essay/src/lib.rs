//! Home's record of the cards essays (CARDS7, #854, paulgsc/some-ui#2979):
//! `cards_essay`, each essay once by its key, append-only. The rules are
//! `20261114000100_create_cards_essay.up.sql`'s.

use sha2::{Digest, Sha256};
use sqlx::SqliteConnection;

/// An essay as the repository served it.
#[derive(Debug, Clone, Copy)]
pub struct Essay<'a> {
	/// `<slug>-<YYYY-MM-DD>`: its file's name.
	pub key: &'a str,
	/// The key's day, `YYYY-MM-DD`.
	pub day: &'a str,
	/// The cut list's bytes, exactly as served.
	pub body: &'a [u8],
	/// ISO-8601 UTC, when home took it.
	pub taken_at: &'a str,
}

/// What [`keep`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
	/// The essay is new to home, and stored.
	Added,
	/// Home already holds an essay of that key: nothing is written.
	Held,
}

/// An essay home holds, and when it took it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldEssay {
	pub key: String,
	pub day: String,
	/// ISO-8601 UTC, as [`Essay::taken_at`] stored it.
	pub taken_at: String,
}

/// Lowercase hex SHA-256 of `bytes`.
fn sha256_hex(bytes: &[u8]) -> String {
	hex::encode(Sha256::digest(bytes))
}

/// Stores `essay` unless home already holds one of its key.
///
/// # Errors
/// A storage failure, or an essay the table's checks refuse (a key that is
/// not `<slug>-<YYYY-MM-DD>` of its day).
pub async fn keep(conn: &mut SqliteConnection, essay: &Essay<'_>) -> Result<Kept, sqlx::Error> {
	let sha256 = sha256_hex(essay.body);
	let size = i64::try_from(essay.body.len()).unwrap_or(i64::MAX);
	let done = sqlx::query!(
		"INSERT INTO cards_essay (essay_key, day, sha256, size, body, taken_at) VALUES (?, ?, ?, ?, ?, ?)
		 ON CONFLICT (essay_key) DO NOTHING",
		essay.key,
		essay.day,
		sha256,
		size,
		essay.body,
		essay.taken_at,
	)
	.execute(&mut *conn)
	.await?;
	Ok(if done.rows_affected() == 1 { Kept::Added } else { Kept::Held })
}

/// Every essay home holds, oldest day first (then by key): the order the
/// render worker takes them in.
///
/// # Errors
/// A storage failure.
pub async fn held(conn: &mut SqliteConnection) -> Result<Vec<HeldEssay>, sqlx::Error> {
	sqlx::query_as!(
		HeldEssay,
		r#"SELECT essay_key AS "key!", day AS "day!", taken_at AS "taken_at!" FROM cards_essay ORDER BY day, essay_key"#
	)
	.fetch_all(&mut *conn)
	.await
}

/// The bytes home holds for the essay `key`, or `None`.
///
/// # Errors
/// A storage failure.
pub async fn body(conn: &mut SqliteConnection, key: &str) -> Result<Option<Vec<u8>>, sqlx::Error> {
	sqlx::query_scalar!("SELECT body FROM cards_essay WHERE essay_key = ?", key)
		.fetch_optional(&mut *conn)
		.await
}

/// Whether home holds an essay of `key`.
///
/// # Errors
/// A storage failure.
pub async fn holds(conn: &mut SqliteConnection, key: &str) -> Result<bool, sqlx::Error> {
	Ok(
		sqlx::query_scalar!(r#"SELECT 1 AS "one!: i64" FROM cards_essay WHERE essay_key = ?"#, key)
			.fetch_optional(&mut *conn)
			.await?
			.is_some(),
	)
}

/// Each essay's title, by key: its cut list's top-level `title` when that is
/// a string, read inside the database so a list never loads the bodies.
/// `None` for an essay whose cut list has none (or one that is not text).
///
/// # Errors
/// A storage failure.
pub async fn titles(conn: &mut SqliteConnection) -> Result<Vec<(String, Option<String>)>, sqlx::Error> {
	let rows = sqlx::query!(
		r#"SELECT essay_key AS "key!",
		          CASE WHEN json_valid(CAST(body AS TEXT))
		               THEN CASE WHEN json_type(CAST(body AS TEXT), '$.title') = 'text'
		                         THEN json_extract(CAST(body AS TEXT), '$.title') END
		          END AS "title?: String"
		   FROM cards_essay"#
	)
	.fetch_all(&mut *conn)
	.await?;
	Ok(rows.into_iter().map(|row| (row.key, row.title)).collect())
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

	fn essay<'a>(key: &'a str, body: &'a [u8], taken_at: &'a str) -> Essay<'a> {
		Essay {
			key,
			day: &key[key.len() - 10..],
			body,
			taken_at,
		}
	}

	#[tokio::test]
	async fn an_essay_is_kept_once_as_served() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		let first = br#"{"schema":"some-ui.cut-list/2"}"#;
		let at = "2026-11-14T08:00:00.000Z";
		assert_eq!(keep(&mut conn, &essay("rust-scratchpad-2026-11-14", first, at)).await.unwrap(), Kept::Added);
		let edited = br#"{"schema":"some-ui.cut-list/2","edited":true}"#;
		let later = "2026-11-14T09:00:00.000Z";
		assert_eq!(
			keep(&mut conn, &essay("rust-scratchpad-2026-11-14", edited, later)).await.unwrap(),
			Kept::Held,
			"an edit is not taken"
		);
		assert_eq!(body(&mut conn, "rust-scratchpad-2026-11-14").await.unwrap().as_deref(), Some(&first[..]));
		assert_eq!(keep(&mut conn, &essay("zfs-2026-11-02", edited, later)).await.unwrap(), Kept::Added);
		assert_eq!(keep(&mut conn, &essay("nixos-upgrade-2026-11-14", edited, later)).await.unwrap(), Kept::Added);

		let keys: Vec<String> = held(&mut conn).await.unwrap().into_iter().map(|held| held.key).collect();
		assert_eq!(keys, ["zfs-2026-11-02", "nixos-upgrade-2026-11-14", "rust-scratchpad-2026-11-14"], "by day, not by name");
	}

	#[tokio::test]
	async fn an_essay_is_held_by_its_key_and_titled_by_its_cut_list() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		let at = "2026-11-14T08:00:00.000Z";
		keep(&mut conn, &essay("nixos-upgrade-2026-11-14", br#"{"title":"A machine upgrades itself"}"#, at))
			.await
			.unwrap();
		keep(&mut conn, &essay("zfs-2026-11-02", br#"{"schema":"some-ui.cut-list/2"}"#, at)).await.unwrap();
		keep(&mut conn, &essay("btrfs-2026-11-03", br#"{"title":7}"#, at)).await.unwrap();
		keep(&mut conn, &essay("ext4-2026-11-04", b"not json", at)).await.unwrap();
		assert!(holds(&mut conn, "zfs-2026-11-02").await.unwrap());
		assert!(!holds(&mut conn, "zfs-2026-11-03").await.unwrap());
		let mut titles = titles(&mut conn).await.unwrap();
		titles.sort();
		assert_eq!(
			titles,
			[
				(String::from("btrfs-2026-11-03"), None),
				(String::from("ext4-2026-11-04"), None),
				(String::from("nixos-upgrade-2026-11-14"), Some(String::from("A machine upgrades itself"))),
				(String::from("zfs-2026-11-02"), None),
			],
			"a title only where the cut list has one that is text"
		);
	}

	#[tokio::test]
	async fn a_key_that_is_not_its_day_is_refused() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		for key in ["NixOS-2026-11-14", "../etc-2026-11-14", "x_y-2026-11-14"] {
			assert!(keep(&mut conn, &essay(key, b"{}", "t")).await.is_err(), "{key} is refused");
		}
		let mismatched = Essay {
			key: "nixos-2026-11-14",
			day: "2026-11-15",
			body: b"{}",
			taken_at: "t",
		};
		assert!(keep(&mut conn, &mismatched).await.is_err(), "a day that is not the key's is refused");
	}

	#[tokio::test]
	async fn no_kept_essay_is_rewritten_or_deleted() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		keep(&mut conn, &essay("nixos-upgrade-2026-11-14", b"{}", "2026-11-14T08:00:00.000Z")).await.unwrap();
		let update = sqlx::query("UPDATE cards_essay SET body = x'00'").execute(&mut *conn).await;
		assert!(update.is_err(), "UPDATE is refused");
		let delete = sqlx::query("DELETE FROM cards_essay").execute(&mut *conn).await;
		assert!(delete.is_err(), "DELETE is refused");
		assert_eq!(held(&mut conn).await.unwrap().len(), 1);
	}
}
