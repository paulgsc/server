//! Home's record of Playoff Delta's fills (paulgsc/some-ui#2647, FEED4):
//! `pd_fill`, each fill once by its episode (season, team, week), with the
//! season file it was taken beside, append-only. The rules are
//! `20261108000200_create_pd_fill.up.sql`'s.

pub mod video;

use sha2::{Digest, Sha256};
use sqlx::SqliteConnection;

/// A fill and its season, as the repository served them.
#[derive(Debug, Clone, Copy)]
pub struct Fill<'a> {
	/// The season's label, `YYYY`.
	pub season: &'a str,
	/// Whose side the season is read from: `SF`.
	pub team: &'a str,
	/// The week, 0 for the legend.
	pub week: i64,
	/// The format version the fill was written for.
	pub version: i64,
	/// The fill's bytes, exactly as served.
	pub body: &'a [u8],
	/// The season file's bytes, exactly as served.
	pub season_body: &'a [u8],
	/// ISO-8601 UTC, when home took it.
	pub taken_at: &'a str,
}

/// What [`keep`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
	/// The fill is new to home, and stored.
	Added,
	/// Home already holds a fill of that episode: nothing is written.
	Held,
}

/// A fill home holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldFill {
	pub season: String,
	pub team: String,
	pub week: i64,
	pub version: i64,
	/// Lowercase hex SHA-256 of the fill's bytes home holds.
	pub sha256: String,
	/// ISO-8601 UTC, when home took it.
	pub taken_at: String,
}

impl HeldFill {
	/// The episode's key, `<season>-<team>-<NN>` (`2026-SF-08`): the name of
	/// its folders on home's drive. Each part is one the table's checks
	/// hold to digits or capitals, so the key is one plain path segment.
	#[must_use]
	pub fn key(&self) -> String {
		key(&self.season, &self.team, self.week)
	}
}

/// An episode's key, as [`HeldFill::key`] writes it.
#[must_use]
pub fn key(season: &str, team: &str, week: i64) -> String {
	let mut key = String::from(season) + "-" + team + "-";
	if week < 10 {
		key.push('0');
	}
	key + &week.to_string()
}

/// The episode a key names, `(season, team, week)`, when it is one
/// [`key`] writes: four digits, two or three capitals, two digits.
#[must_use]
pub fn parse_key(key: &str) -> Option<(String, String, i64)> {
	let mut parts = key.split('-');
	let (season, team, week) = (parts.next()?, parts.next()?, parts.next()?);
	let digits = |text: &str, len: usize| text.len() == len && text.bytes().all(|byte| byte.is_ascii_digit());
	let capitals = (2..=3).contains(&team.len()) && team.bytes().all(|byte| byte.is_ascii_uppercase());
	if parts.next().is_some() || !digits(season, 4) || !capitals || !digits(week, 2) {
		return None;
	}
	Some((season.to_owned(), team.to_owned(), week.parse().ok()?))
}

/// Lowercase hex SHA-256 of `bytes`.
fn sha256_hex(bytes: &[u8]) -> String {
	hex::encode(Sha256::digest(bytes))
}

/// Stores `fill` unless home already holds a fill of its episode.
///
/// # Errors
/// A storage failure, or a fill the table's checks refuse (a season that is
/// not four digits, a team that is not two or three capitals, a week past
/// 99, a version below 1).
pub async fn keep(conn: &mut SqliteConnection, fill: &Fill<'_>) -> Result<Kept, sqlx::Error> {
	let sha256 = sha256_hex(fill.body);
	let size = i64::try_from(fill.body.len()).unwrap_or(i64::MAX);
	let season_sha256 = sha256_hex(fill.season_body);
	let season_size = i64::try_from(fill.season_body.len()).unwrap_or(i64::MAX);
	let done = sqlx::query!(
		"INSERT INTO pd_fill (season, team, week, version, sha256, size, body, season_sha256, season_size, season_body, taken_at)
		 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
		 ON CONFLICT (season, team, week) DO NOTHING",
		fill.season,
		fill.team,
		fill.week,
		fill.version,
		sha256,
		size,
		fill.body,
		season_sha256,
		season_size,
		fill.season_body,
		fill.taken_at,
	)
	.execute(&mut *conn)
	.await?;
	Ok(if done.rows_affected() == 1 { Kept::Added } else { Kept::Held })
}

/// Every fill home holds, by season, team and week.
///
/// # Errors
/// A storage failure.
pub async fn held(conn: &mut SqliteConnection) -> Result<Vec<HeldFill>, sqlx::Error> {
	sqlx::query_as!(
		HeldFill,
		r#"SELECT season AS "season!", team AS "team!", week AS "week!: i64", version AS "version!: i64", sha256 AS "sha256!", taken_at AS "taken_at!"
		   FROM pd_fill ORDER BY season, team, week"#
	)
	.fetch_all(&mut *conn)
	.await
}

/// The bytes home holds for an episode: its fill and its season, or `None`.
///
/// # Errors
/// A storage failure.
pub async fn bodies(conn: &mut SqliteConnection, season: &str, team: &str, week: i64) -> Result<Option<(Vec<u8>, Vec<u8>)>, sqlx::Error> {
	let row = sqlx::query!("SELECT body, season_body FROM pd_fill WHERE season = ? AND team = ? AND week = ?", season, team, week)
		.fetch_optional(&mut *conn)
		.await?;
	Ok(row.map(|row| (row.body, row.season_body)))
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

	fn fill<'a>(week: i64, body: &'a [u8], season_body: &'a [u8], taken_at: &'a str) -> Fill<'a> {
		Fill {
			season: "2026",
			team: "SF",
			week,
			version: 2,
			body,
			season_body,
			taken_at,
		}
	}

	#[tokio::test]
	async fn a_fill_is_kept_once_as_served_with_its_season() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		let (first, season) = (br#"{"week":8}"#, br#"{"label":"2026"}"#);
		let at = "2026-11-10T19:00:00.000Z";
		assert_eq!(keep(&mut conn, &fill(8, first, season, at)).await.unwrap(), Kept::Added);
		let edited = br#"{"week":8,"edited":true}"#;
		let later = "2026-11-10T20:00:00.000Z";
		assert_eq!(keep(&mut conn, &fill(8, edited, b"{}", later)).await.unwrap(), Kept::Held, "an edit is not taken");
		assert_eq!(bodies(&mut conn, "2026", "SF", 8).await.unwrap(), Some((first.to_vec(), season.to_vec())));
		assert_eq!(bodies(&mut conn, "2026", "SF", 9).await.unwrap(), None);
		assert_eq!(keep(&mut conn, &fill(0, edited, season, later)).await.unwrap(), Kept::Added);

		let listed = held(&mut conn).await.unwrap();
		let keys: Vec<String> = listed.iter().map(HeldFill::key).collect();
		assert_eq!(keys, ["2026-SF-00", "2026-SF-08"], "by week, the legend first");
		assert_eq!(parse_key("2026-SF-08"), Some(("2026".to_owned(), "SF".to_owned(), 8)));
		for bad in ["2026-SF-8", "2026-sf-08", "26-SF-08", "2026-SF-08-x", "../SF-08", "2026-SFOO-08"] {
			assert_eq!(parse_key(bad), None, "{bad}");
		}
		assert_eq!((listed[1].sha256.as_str(), listed[1].taken_at.as_str()), (sha256_hex(first).as_str(), at));
		assert_eq!(listed[1].version, 2);
	}

	#[tokio::test]
	async fn no_kept_fill_is_rewritten_or_deleted_and_a_bad_episode_is_refused() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		keep(&mut conn, &fill(8, b"{}", b"{}", "2026-11-10T19:00:00.000Z")).await.unwrap();
		assert!(sqlx::query("UPDATE pd_fill SET body = x'00'").execute(&mut *conn).await.is_err(), "UPDATE is refused");
		assert!(sqlx::query("DELETE FROM pd_fill").execute(&mut *conn).await.is_err(), "DELETE is refused");
		for (season, team, week) in [("26", "SF", 1), ("2026", "../x", 1), ("2026", "sf", 1), ("2026", "SF", 100)] {
			let bad = Fill {
				season,
				team,
				week,
				..fill(1, b"{}", b"{}", "t")
			};
			assert!(keep(&mut conn, &bad).await.is_err(), "{season} {team} {week}");
		}
		assert_eq!(held(&mut conn).await.unwrap().len(), 1);
	}
}
