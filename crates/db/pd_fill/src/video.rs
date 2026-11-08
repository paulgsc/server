//! Playoff Delta's videos once they are on `YouTube` (FEED4).
//!
//! Each episode's `YouTube` link, and a receipt for each step of an episode
//! the owner let go from My data, by the call's rules and with the call's
//! types (`call_week_repo::video`): a link is kept once per episode and
//! video, the newest standing; an episode goes video first, then the rest,
//! each with one receipt kept for good. An episode is named by its key,
//! `<season>-<team>-<NN>` ([`crate::key`]); a key that isn't one names no
//! episode. The rules are `20261108000300_create_pd_video.up.sql`'s; this
//! module is the one that writes `pd_video_link` and `pd_video_deletion`.

use crate::{key, parse_key};
use call_week_repo::video::{Link, Receipt, Recorded, Step};
use sqlx::SqliteConnection;
use std::collections::BTreeMap;

/// Records `youtube_id` as the episode's link, once.
///
/// # Errors
/// A storage failure, or an id the table's checks refuse.
pub async fn record_link(conn: &mut SqliteConnection, episode: &str, youtube_id: &str, now: &str) -> Result<Recorded, sqlx::Error> {
	let Some((season, team, week)) = parse_key(episode) else {
		return Ok(Recorded::NoWeek);
	};
	let held = sqlx::query_scalar!(r#"SELECT 1 AS "one!: i64" FROM pd_fill WHERE season = ? AND team = ? AND week = ?"#, season, team, week)
		.fetch_optional(&mut *conn)
		.await?;
	if held.is_none() {
		return Ok(Recorded::NoWeek);
	}
	let done = sqlx::query!(
		"INSERT INTO pd_video_link (season, team, week, youtube_id, recorded_at) VALUES (?, ?, ?, ?, ?)
		 ON CONFLICT (season, team, week, youtube_id) DO NOTHING",
		season,
		team,
		week,
		youtube_id,
		now,
	)
	.execute(&mut *conn)
	.await?;
	Ok(if done.rows_affected() == 1 { Recorded::Added } else { Recorded::Held })
}

/// Every episode's link: the newest recorded of each, by key.
///
/// # Errors
/// A storage failure.
pub async fn links(conn: &mut SqliteConnection) -> Result<BTreeMap<String, Link>, sqlx::Error> {
	let rows = sqlx::query!(
		r#"SELECT season AS "season!", team AS "team!", week AS "week!: i64", youtube_id AS "youtube_id!", recorded_at AS "recorded_at!"
		   FROM pd_video_link ORDER BY season, team, week, recorded_at, youtube_id"#
	)
	.fetch_all(&mut *conn)
	.await?;
	Ok(
		rows
			.into_iter()
			.map(|row| {
				(
					key(&row.season, &row.team, row.week),
					Link {
						youtube_id: row.youtube_id,
						recorded_at: row.recorded_at,
					},
				)
			})
			.collect(),
	)
}

/// Every receipt, by episode and step; each [`Receipt::from`] is the
/// episode's key.
///
/// # Errors
/// A storage failure, or a step the table holds that this does not know.
pub async fn receipts(conn: &mut SqliteConnection) -> Result<Vec<Receipt>, sqlx::Error> {
	let rows = sqlx::query!(
		r#"SELECT season AS "season!", team AS "team!", week AS "week!: i64", step AS "step!", youtube_id AS "youtube_id!",
		          size AS "size!: i64", deleted_at AS "deleted_at!"
		   FROM pd_video_deletion ORDER BY season, team, week, step DESC"#
	)
	.fetch_all(&mut *conn)
	.await?;
	rows
		.into_iter()
		.map(|row| {
			let step = Step::parse(&row.step).ok_or_else(|| sqlx::Error::Decode((String::from("unknown step ") + &row.step).into()))?;
			Ok(Receipt {
				from: key(&row.season, &row.team, row.week),
				step,
				youtube_id: row.youtube_id,
				size: row.size,
				deleted_at: row.deleted_at,
			})
		})
		.collect()
}

/// The receipt of an episode's step, if it was let go.
///
/// # Errors
/// A storage failure.
pub async fn receipt(conn: &mut SqliteConnection, episode: &str, step: Step) -> Result<Option<Receipt>, sqlx::Error> {
	let Some((season, team, week)) = parse_key(episode) else {
		return Ok(None);
	};
	let name = step.as_str();
	let row = sqlx::query!(
		r#"SELECT youtube_id AS "youtube_id!", size AS "size!: i64", deleted_at AS "deleted_at!"
		   FROM pd_video_deletion WHERE season = ? AND team = ? AND week = ? AND step = ?"#,
		season,
		team,
		week,
		name,
	)
	.fetch_optional(&mut *conn)
	.await?;
	Ok(row.map(|row| Receipt {
		from: episode.to_owned(),
		step,
		youtube_id: row.youtube_id,
		size: row.size,
		deleted_at: row.deleted_at,
	}))
}

/// Writes a step's receipt; its [`Receipt::from`] is the episode's key.
///
/// # Errors
/// A storage failure, or what the table refuses: a key that names no
/// episode, a second receipt for the step, a link the episode never had, or
/// the rest before the video.
pub async fn write_receipt(conn: &mut SqliteConnection, receipt: &Receipt) -> Result<(), sqlx::Error> {
	let Some((season, team, week)) = parse_key(&receipt.from) else {
		return Err(sqlx::Error::Protocol(String::from("not an episode's key")));
	};
	let step = receipt.step.as_str();
	sqlx::query!(
		"INSERT INTO pd_video_deletion (season, team, week, step, youtube_id, size, deleted_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
		season,
		team,
		week,
		step,
		receipt.youtube_id,
		receipt.size,
		receipt.deleted_at,
	)
	.execute(&mut *conn)
	.await?;
	Ok(())
}

/// The episodes whose video was let go, by key: the render worker is never
/// given them again.
///
/// # Errors
/// A storage failure.
pub async fn let_go(conn: &mut SqliteConnection) -> Result<Vec<String>, sqlx::Error> {
	let rows = sqlx::query!(
		r#"SELECT season AS "season!", team AS "team!", week AS "week!: i64" FROM pd_video_deletion WHERE step = 'video'
		   ORDER BY season, team, week"#
	)
	.fetch_all(&mut *conn)
	.await?;
	Ok(rows.into_iter().map(|row| key(&row.season, &row.team, row.week)).collect())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{keep, Fill};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	const ID: &str = "dQw4w9WgXcQ";
	const EPISODE: &str = "2026-SF-09";

	async fn hold(conn: &mut SqliteConnection) {
		let fill = Fill {
			season: "2026",
			team: "SF",
			week: 9,
			version: 2,
			body: b"{}",
			season_body: b"{}",
			taken_at: "2026-11-10T20:00:00.000Z",
		};
		keep(conn, &fill).await.unwrap();
	}

	#[tokio::test]
	async fn an_episode_goes_video_first_then_the_rest_once_its_link_is_recorded_each_kept_for_good() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(record_link(&mut conn, EPISODE, ID, "t").await.unwrap(), Recorded::NoWeek);
		assert_eq!(record_link(&mut conn, "../x", ID, "t").await.unwrap(), Recorded::NoWeek);
		hold(&mut conn).await;
		assert_eq!(record_link(&mut conn, EPISODE, ID, "2026-11-11T10:00:00.000Z").await.unwrap(), Recorded::Added);
		assert_eq!(record_link(&mut conn, EPISODE, ID, "2026-11-11T11:00:00.000Z").await.unwrap(), Recorded::Held);
		assert_eq!(links(&mut conn).await.unwrap()[EPISODE].youtube_id, ID);

		let made = |step, size| Receipt {
			from: EPISODE.to_owned(),
			step,
			youtube_id: ID.to_owned(),
			size,
			deleted_at: "2026-11-11T12:00:00.000Z".to_owned(),
		};
		assert!(write_receipt(&mut conn, &made(Step::Rest, 7)).await.is_err(), "the rest goes only after the video");
		write_receipt(&mut conn, &made(Step::Video, 90)).await.unwrap();
		assert!(write_receipt(&mut conn, &made(Step::Video, 90)).await.is_err(), "one receipt a step");
		write_receipt(&mut conn, &made(Step::Rest, 7)).await.unwrap();
		assert_eq!(receipts(&mut conn).await.unwrap(), [made(Step::Video, 90), made(Step::Rest, 7)]);
		assert_eq!(receipt(&mut conn, EPISODE, Step::Video).await.unwrap(), Some(made(Step::Video, 90)));
		assert_eq!(let_go(&mut conn).await.unwrap(), [EPISODE]);
		for table in ["pd_video_link", "pd_video_deletion"] {
			let update = String::from("UPDATE ") + table + " SET youtube_id = 'bbbbbbbbbbb'";
			assert!(sqlx::query(&update).execute(&mut *conn).await.is_err(), "{table} UPDATE");
			let delete = String::from("DELETE FROM ") + table;
			assert!(sqlx::query(&delete).execute(&mut *conn).await.is_err(), "{table} DELETE");
		}
	}
}
