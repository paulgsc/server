//! The weekly call's videos once they are on `YouTube` (ST38).
//!
//! Each week's `YouTube` link, and a receipt for each step of a week the
//! owner let go from My data (paulgsc/some-ui#2661). The rules are
//! `20261107000100_create_call_video.up.sql`'s; this module is the one that
//! writes `call_video_link` and `call_video_deletion`.
//!
//! - **A link is a `YouTube` video's id,** read from a link the owner pasted
//!   ([`youtube_id`]), and kept once per week and video; the newest recorded
//!   is the week's link ([`links`]). Nothing is rewritten.
//! - **A week is let go in two steps,** the video and then the rest
//!   ([`Step`]), each with one receipt kept for good. The video's step needs
//!   a link, and the rest's the video's receipt; the table's own trigger
//!   holds the second as well.
//! - **What goes on the drive is `file_host`'s** (`call_weeks::videos`),
//!   which writes the receipt in the transaction it deletes under.

use sqlx::SqliteConnection;
use std::collections::BTreeMap;

/// A step of letting a week go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Step {
	/// Its `call.mp4`.
	Video,
	/// The rest of its folder: takes, stills, cut list, the week's file.
	Rest,
}

impl Step {
	/// The step as the table and the routes name it.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Video => "video",
			Self::Rest => "rest",
		}
	}

	/// The step a name names, if any.
	#[must_use]
	pub fn parse(name: &str) -> Option<Self> {
		match name {
			"video" => Some(Self::Video),
			"rest" => Some(Self::Rest),
			_ => None,
		}
	}
}

/// A step's receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
	pub from: String,
	pub step: Step,
	/// The week's link when it was let go.
	pub youtube_id: String,
	/// How many bytes went.
	pub size: i64,
	/// ISO-8601 UTC.
	pub deleted_at: String,
}

/// A week's link as home recorded it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
	pub youtube_id: String,
	/// ISO-8601 UTC.
	pub recorded_at: String,
}

/// The `YouTube` video a pasted link names: its eleven-character id.
///
/// From `youtube.com/watch?v=<id>`, `youtu.be/<id>` or
/// `youtube.com/shorts/<id>` (with or without `https://`, `www.` or `m.`,
/// and with any further query or fragment). A channel, a playlist or
/// anything else names none.
#[must_use]
pub fn youtube_id(link: &str) -> Option<String> {
	let link = link.trim();
	let rest = link.strip_prefix("https://").or_else(|| link.strip_prefix("http://")).unwrap_or(link);
	let (host, path) = rest.split_once('/')?;
	let host = host.strip_prefix("www.").or_else(|| host.strip_prefix("m.")).unwrap_or(host);
	let candidate = match host {
		"youtu.be" => path,
		"youtube.com" => {
			if let Some(short) = path.strip_prefix("shorts/") {
				short
			} else {
				let query = path.strip_prefix("watch?")?;
				query.split('&').find_map(|pair| pair.strip_prefix("v="))?
			}
		}
		_ => return None,
	};
	let id = candidate.split(['?', '&', '#', '/']).next()?;
	let valid = id.len() == 11 && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
	valid.then(|| id.to_owned())
}

/// A video's link, as home gives it back.
#[must_use]
pub fn link_of(youtube_id: &str) -> String {
	String::from("https://youtu.be/") + youtube_id
}

/// What [`record_link`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
	/// The link is new for the week, and stored.
	Added,
	/// Home already holds this link for the week: nothing is written.
	Held,
	/// Home holds no week of that first day.
	NoWeek,
}

/// Records `youtube_id` as the week's link, once.
///
/// # Errors
/// A storage failure, or an id the table's checks refuse.
pub async fn record_link(conn: &mut SqliteConnection, from: &str, youtube_id: &str, now: &str) -> Result<Recorded, sqlx::Error> {
	let held = sqlx::query_scalar!(r#"SELECT 1 AS "one!: i64" FROM call_week WHERE week_from = ?"#, from)
		.fetch_optional(&mut *conn)
		.await?;
	if held.is_none() {
		return Ok(Recorded::NoWeek);
	}
	let done = sqlx::query!(
		"INSERT INTO call_video_link (week_from, youtube_id, recorded_at) VALUES (?, ?, ?) ON CONFLICT (week_from, youtube_id) DO NOTHING",
		from,
		youtube_id,
		now,
	)
	.execute(&mut *conn)
	.await?;
	Ok(if done.rows_affected() == 1 { Recorded::Added } else { Recorded::Held })
}

/// Every week's link: the newest recorded of each, by week.
///
/// # Errors
/// A storage failure.
pub async fn links(conn: &mut SqliteConnection) -> Result<BTreeMap<String, Link>, sqlx::Error> {
	let rows = sqlx::query!(
		r#"SELECT week_from AS "from!", youtube_id AS "youtube_id!", recorded_at AS "recorded_at!"
		   FROM call_video_link ORDER BY week_from, recorded_at, youtube_id"#
	)
	.fetch_all(&mut *conn)
	.await?;
	Ok(
		rows
			.into_iter()
			.map(|row| {
				(
					row.from,
					Link {
						youtube_id: row.youtube_id,
						recorded_at: row.recorded_at,
					},
				)
			})
			.collect(),
	)
}

/// Every receipt, by week and step.
///
/// # Errors
/// A storage failure, or a step the table holds that this does not know.
pub async fn receipts(conn: &mut SqliteConnection) -> Result<Vec<Receipt>, sqlx::Error> {
	let rows = sqlx::query!(
		r#"SELECT week_from AS "from!", step AS "step!", youtube_id AS "youtube_id!", size AS "size!: i64", deleted_at AS "deleted_at!"
		   FROM call_video_deletion ORDER BY week_from, step DESC"#
	)
	.fetch_all(&mut *conn)
	.await?;
	rows
		.into_iter()
		.map(|row| {
			let step = Step::parse(&row.step).ok_or_else(|| sqlx::Error::Decode((String::from("unknown step ") + &row.step).into()))?;
			Ok(Receipt {
				from: row.from,
				step,
				youtube_id: row.youtube_id,
				size: row.size,
				deleted_at: row.deleted_at,
			})
		})
		.collect()
}

/// The receipt of a week's step, if it was let go.
///
/// # Errors
/// A storage failure.
pub async fn receipt(conn: &mut SqliteConnection, from: &str, step: Step) -> Result<Option<Receipt>, sqlx::Error> {
	let name = step.as_str();
	let row = sqlx::query!(
		r#"SELECT youtube_id AS "youtube_id!", size AS "size!: i64", deleted_at AS "deleted_at!"
		   FROM call_video_deletion WHERE week_from = ? AND step = ?"#,
		from,
		name,
	)
	.fetch_optional(&mut *conn)
	.await?;
	Ok(row.map(|row| Receipt {
		from: from.to_owned(),
		step,
		youtube_id: row.youtube_id,
		size: row.size,
		deleted_at: row.deleted_at,
	}))
}

/// Writes a step's receipt.
///
/// # Errors
/// A storage failure, or what the table refuses: a second receipt for the
/// step, a link the week never had, or the rest before the video.
pub async fn write_receipt(conn: &mut SqliteConnection, receipt: &Receipt) -> Result<(), sqlx::Error> {
	let step = receipt.step.as_str();
	sqlx::query!(
		"INSERT INTO call_video_deletion (week_from, step, youtube_id, size, deleted_at) VALUES (?, ?, ?, ?, ?)",
		receipt.from,
		step,
		receipt.youtube_id,
		receipt.size,
		receipt.deleted_at,
	)
	.execute(&mut *conn)
	.await?;
	Ok(())
}

/// The weeks whose video was let go, oldest first: the render worker is
/// never given them again.
///
/// # Errors
/// A storage failure.
pub async fn let_go(conn: &mut SqliteConnection) -> Result<Vec<String>, sqlx::Error> {
	sqlx::query_scalar!(r#"SELECT week_from AS "from!" FROM call_video_deletion WHERE step = 'video' ORDER BY week_from"#)
		.fetch_all(&mut *conn)
		.await
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{keep, Week};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	const ID: &str = "dQw4w9WgXcQ";

	#[test]
	fn a_youtube_video_link_names_its_id_and_nothing_else_names_one() {
		for link in [
			"https://www.youtube.com/watch?v=dQw4w9WgXcQ",
			"https://youtube.com/watch?feature=share&v=dQw4w9WgXcQ&t=42",
			"https://m.youtube.com/watch?v=dQw4w9WgXcQ#comments",
			"youtube.com/watch?v=dQw4w9WgXcQ",
			"https://youtu.be/dQw4w9WgXcQ",
			" https://youtu.be/dQw4w9WgXcQ?si=abc ",
			"https://www.youtube.com/shorts/dQw4w9WgXcQ",
		] {
			assert_eq!(youtube_id(link).as_deref(), Some(ID), "{link}");
		}
		for link in [
			"",
			"https://www.youtube.com/@paulgsc",
			"https://www.youtube.com/playlist?list=PL1234567890",
			"https://youtu.be/short",
			"https://youtu.be/dQw4w9WgXcQ1",
			"https://example.com/watch?v=dQw4w9WgXcQ",
			"https://youtube.com.evil.example/watch?v=dQw4w9WgXcQ",
			"https://youtu.be/dQw4w9Wg<cQ",
		] {
			assert_eq!(youtube_id(link), None, "{link}");
		}
		assert_eq!(link_of(ID), "https://youtu.be/dQw4w9WgXcQ");
	}

	#[tokio::test]
	async fn a_link_is_recorded_once_and_the_newest_stands() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(record_link(&mut conn, "2026-10-30", ID, "2026-11-07T10:00:00.000Z").await.unwrap(), Recorded::NoWeek);
		keep(
			&mut conn,
			&Week {
				from: "2026-10-30",
				to: "2026-11-05",
				body: b"{}",
				taken_at: "2026-11-06T08:00:00.000Z",
			},
		)
		.await
		.unwrap();
		assert_eq!(record_link(&mut conn, "2026-10-30", ID, "2026-11-07T10:00:00.000Z").await.unwrap(), Recorded::Added);
		assert_eq!(record_link(&mut conn, "2026-10-30", ID, "2026-11-07T11:00:00.000Z").await.unwrap(), Recorded::Held);
		assert_eq!(
			record_link(&mut conn, "2026-10-30", "aaaaaaaaaaa", "2026-11-07T12:00:00.000Z").await.unwrap(),
			Recorded::Added
		);
		let links = links(&mut conn).await.unwrap();
		assert_eq!(links["2026-10-30"].youtube_id, "aaaaaaaaaaa", "the newest recorded stands");
		assert!(record_link(&mut conn, "2026-10-30", "not an id", "2026-11-07T12:00:00.000Z").await.is_err());
		assert!(sqlx::query("UPDATE call_video_link SET youtube_id = 'bbbbbbbbbbb'").execute(&mut *conn).await.is_err());
		assert!(sqlx::query("DELETE FROM call_video_link").execute(&mut *conn).await.is_err());
	}

	#[tokio::test]
	async fn a_week_goes_video_first_then_the_rest_each_once_with_a_receipt_kept_for_good() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		keep(
			&mut conn,
			&Week {
				from: "2026-10-30",
				to: "2026-11-05",
				body: b"{}",
				taken_at: "2026-11-06T08:00:00.000Z",
			},
		)
		.await
		.unwrap();
		let receipt = |step, size| Receipt {
			from: "2026-10-30".to_owned(),
			step,
			youtube_id: ID.to_owned(),
			size,
			deleted_at: "2026-11-07T12:00:00.000Z".to_owned(),
		};
		assert!(write_receipt(&mut conn, &receipt(Step::Video, 95)).await.is_err(), "no link, no receipt");
		record_link(&mut conn, "2026-10-30", ID, "2026-11-07T10:00:00.000Z").await.unwrap();
		assert!(write_receipt(&mut conn, &receipt(Step::Rest, 17)).await.is_err(), "the rest goes only after the video");
		write_receipt(&mut conn, &receipt(Step::Video, 95)).await.unwrap();
		assert!(write_receipt(&mut conn, &receipt(Step::Video, 95)).await.is_err(), "one receipt a step");
		write_receipt(&mut conn, &receipt(Step::Rest, 17)).await.unwrap();

		assert_eq!(receipt_of(&mut conn, Step::Video).await, Some(receipt(Step::Video, 95)));
		assert_eq!(receipts(&mut conn).await.unwrap(), [receipt(Step::Video, 95), receipt(Step::Rest, 17)]);
		assert_eq!(let_go(&mut conn).await.unwrap(), ["2026-10-30"]);
		assert!(sqlx::query("UPDATE call_video_deletion SET size = 0").execute(&mut *conn).await.is_err());
		assert!(sqlx::query("DELETE FROM call_video_deletion").execute(&mut *conn).await.is_err());
	}

	async fn receipt_of(conn: &mut SqliteConnection, step: Step) -> Option<Receipt> {
		receipt(conn, "2026-10-30", step).await.unwrap()
	}
}
