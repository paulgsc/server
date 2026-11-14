//! The weekly call's videos once they are on `YouTube` (ST38).
//!
//! Each part's `YouTube` link, and a receipt for each step of a part the
//! owner let go from My data (paulgsc/some-ui#2661). The rules are
//! `20261114000200_create_call_part.up.sql`'s; this module is the one that
//! writes `call_part_link` and `call_part_deletion`. The tables before parts,
//! `call_video_link` and `call_video_deletion`, are kept whole as the copy
//! that migration made part 1 from, and take no row.
//!
//! - **A week is one video or a series of parts** (paulgsc/some-ui#2918),
//!   each part a video of its own, named by a key ([`split_key`]): the
//!   week's day for part 1, as a week of one video always was, and
//!   `<day>-p<k>` for each later part. Every function here takes and gives
//!   that key, so a week of one video reads exactly as it did.
//! - **A link is a `YouTube` video's id,** read from a link the owner pasted
//!   ([`youtube_id`]), and kept once per part and video; the newest recorded
//!   is the part's link ([`links`]). Nothing is rewritten.
//! - **A part is let go in two steps,** the video and then the rest
//!   ([`Step`]), each with one receipt kept for good, which says how many
//!   parts the week was. The video's step needs a link, and the rest's the
//!   video's receipt; the table's own trigger holds the second as well. A
//!   week is let go whole once every part's video is ([`weeks_let_go`]).
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

/// The most parts a week may be: the tables' own bound.
pub const MOST_PARTS: u32 = 99;

/// The week and part an episode's key names (paulgsc/some-ui#2918).
///
/// A part is named by its week's first day, as a week was before parts: part
/// 1 is the bare day (`2026-11-06`), so a week of one video keeps the key it
/// always had, and each later part adds `-p<k>` (`2026-11-06-p2`). `-p1` and
/// anything past [`MOST_PARTS`] name none. The day is checked by
/// `call_week`'s own table, never here.
#[must_use]
pub fn split_key(key: &str) -> Option<(&str, u32)> {
	let Some((week, part)) = key.split_once("-p") else {
		return Some((key, 1));
	};
	let digits = !part.is_empty() && part.len() <= 2 && part.bytes().all(|byte| byte.is_ascii_digit()) && !part.starts_with('0');
	let part: u32 = if digits { part.parse().ok()? } else { return None };
	(2..=MOST_PARTS).contains(&part).then_some((week, part))
}

/// The key of a week's part: the bare day for part 1, `<day>-p<k>` after.
#[must_use]
pub fn part_key(week: &str, part: u32) -> String {
	if part <= 1 {
		week.to_owned()
	} else {
		let mut key = String::from(week);
		key.push_str("-p");
		key.push_str(&part.to_string());
		key
	}
}

/// What [`record_link`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
	/// The link is new for the part, and stored.
	Added,
	/// Home already holds this link for the part: nothing is written.
	Held,
	/// Home holds no week of that first day, or the key names no part.
	NoWeek,
}

/// Records `youtube_id` as the link of the part `key` names, once.
///
/// # Errors
/// A storage failure, or an id the table's checks refuse.
pub async fn record_link(conn: &mut SqliteConnection, key: &str, youtube_id: &str, now: &str) -> Result<Recorded, sqlx::Error> {
	let Some((from, part)) = split_key(key) else {
		return Ok(Recorded::NoWeek);
	};
	let held = sqlx::query_scalar!(r#"SELECT 1 AS "one!: i64" FROM call_week WHERE week_from = ?"#, from)
		.fetch_optional(&mut *conn)
		.await?;
	if held.is_none() {
		return Ok(Recorded::NoWeek);
	}
	let done = sqlx::query!(
		"INSERT INTO call_part_link (week_from, part, youtube_id, recorded_at) VALUES (?, ?, ?, ?) \
		 ON CONFLICT (week_from, part, youtube_id) DO NOTHING",
		from,
		part,
		youtube_id,
		now,
	)
	.execute(&mut *conn)
	.await?;
	Ok(if done.rows_affected() == 1 { Recorded::Added } else { Recorded::Held })
}

/// Every part's link: the newest recorded of each, by its key.
///
/// # Errors
/// A storage failure.
pub async fn links(conn: &mut SqliteConnection) -> Result<BTreeMap<String, Link>, sqlx::Error> {
	let rows = sqlx::query!(
		r#"SELECT week_from AS "from!", part AS "part!: u32", youtube_id AS "youtube_id!", recorded_at AS "recorded_at!"
		   FROM call_part_link ORDER BY week_from, part, recorded_at, youtube_id"#
	)
	.fetch_all(&mut *conn)
	.await?;
	Ok(
		rows
			.into_iter()
			.map(|row| {
				(
					part_key(&row.from, row.part),
					Link {
						youtube_id: row.youtube_id,
						recorded_at: row.recorded_at,
					},
				)
			})
			.collect(),
	)
}

fn step_of(name: &str) -> Result<Step, sqlx::Error> {
	Step::parse(name).ok_or_else(|| sqlx::Error::Decode((String::from("unknown step ") + name).into()))
}

/// Every receipt, by week, part and step; each names its part by its key.
///
/// # Errors
/// A storage failure, or a step the table holds that this does not know.
pub async fn receipts(conn: &mut SqliteConnection) -> Result<Vec<Receipt>, sqlx::Error> {
	let rows = sqlx::query!(
		r#"SELECT week_from AS "from!", part AS "part!: u32", step AS "step!", youtube_id AS "youtube_id!", size AS "size!: i64",
		          deleted_at AS "deleted_at!"
		   FROM call_part_deletion ORDER BY week_from, part, step DESC"#
	)
	.fetch_all(&mut *conn)
	.await?;
	rows
		.into_iter()
		.map(|row| {
			Ok(Receipt {
				from: part_key(&row.from, row.part),
				step: step_of(&row.step)?,
				youtube_id: row.youtube_id,
				size: row.size,
				deleted_at: row.deleted_at,
			})
		})
		.collect()
}

/// The receipt of a part's step, if it was let go.
///
/// # Errors
/// A storage failure.
pub async fn receipt(conn: &mut SqliteConnection, key: &str, step: Step) -> Result<Option<Receipt>, sqlx::Error> {
	let Some((from, part)) = split_key(key) else {
		return Ok(None);
	};
	let name = step.as_str();
	let row = sqlx::query!(
		r#"SELECT youtube_id AS "youtube_id!", size AS "size!: i64", deleted_at AS "deleted_at!"
		   FROM call_part_deletion WHERE week_from = ? AND part = ? AND step = ?"#,
		from,
		part,
		name,
	)
	.fetch_optional(&mut *conn)
	.await?;
	Ok(row.map(|row| Receipt {
		from: key.to_owned(),
		step,
		youtube_id: row.youtube_id,
		size: row.size,
		deleted_at: row.deleted_at,
	}))
}

/// Writes a step's receipt for the part its `from` names, of a week of `of`
/// parts.
///
/// # Errors
/// A storage failure, or what the table refuses: a key that names no part,
/// a part past `of`, a second receipt for the step, a link the part never
/// had, or the rest before the video.
pub async fn write_receipt(conn: &mut SqliteConnection, receipt: &Receipt, of: u32) -> Result<(), sqlx::Error> {
	let (from, part) = split_key(&receipt.from).ok_or_else(|| sqlx::Error::Protocol(String::from("a call receipt's key names no part")))?;
	let step = receipt.step.as_str();
	sqlx::query!(
		"INSERT INTO call_part_deletion (week_from, part, of, step, youtube_id, size, deleted_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
		from,
		part,
		of,
		step,
		receipt.youtube_id,
		receipt.size,
		receipt.deleted_at,
	)
	.execute(&mut *conn)
	.await?;
	Ok(())
}

/// The parts whose video was let go, by key, oldest week first.
///
/// # Errors
/// A storage failure.
pub async fn let_go(conn: &mut SqliteConnection) -> Result<Vec<String>, sqlx::Error> {
	let rows = sqlx::query!(r#"SELECT week_from AS "from!", part AS "part!: u32" FROM call_part_deletion WHERE step = 'video' ORDER BY week_from, part"#)
		.fetch_all(&mut *conn)
		.await?;
	Ok(rows.into_iter().map(|row| part_key(&row.from, row.part)).collect())
}

/// The weeks let go whole, oldest first: each of the week's parts (as many
/// as the most any of its receipts says it was) has its video's receipt. The render
/// worker is never given them again.
///
/// # Errors
/// A storage failure.
pub async fn weeks_let_go(conn: &mut SqliteConnection) -> Result<Vec<String>, sqlx::Error> {
	sqlx::query_scalar!(
		r#"SELECT week_from AS "from!" FROM call_part_deletion WHERE step = 'video'
		   GROUP BY week_from HAVING COUNT(DISTINCT part) >= MAX(of) ORDER BY week_from"#
	)
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
		assert!(sqlx::query("UPDATE call_part_link SET youtube_id = 'bbbbbbbbbbb'").execute(&mut *conn).await.is_err());
		assert!(sqlx::query("DELETE FROM call_part_link").execute(&mut *conn).await.is_err());
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
		assert!(write_receipt(&mut conn, &receipt(Step::Video, 95), 1).await.is_err(), "no link, no receipt");
		record_link(&mut conn, "2026-10-30", ID, "2026-11-07T10:00:00.000Z").await.unwrap();
		assert!(write_receipt(&mut conn, &receipt(Step::Rest, 17), 1).await.is_err(), "the rest goes only after the video");
		write_receipt(&mut conn, &receipt(Step::Video, 95), 1).await.unwrap();
		assert!(write_receipt(&mut conn, &receipt(Step::Video, 95), 1).await.is_err(), "one receipt a step");
		write_receipt(&mut conn, &receipt(Step::Rest, 17), 1).await.unwrap();

		assert_eq!(receipt_of(&mut conn, Step::Video).await, Some(receipt(Step::Video, 95)));
		assert_eq!(receipts(&mut conn).await.unwrap(), [receipt(Step::Video, 95), receipt(Step::Rest, 17)]);
		assert_eq!(let_go(&mut conn).await.unwrap(), ["2026-10-30"]);
		assert_eq!(weeks_let_go(&mut conn).await.unwrap(), ["2026-10-30"], "a week of one part goes with its video");
		assert!(sqlx::query("UPDATE call_part_deletion SET size = 0").execute(&mut *conn).await.is_err());
		assert!(sqlx::query("DELETE FROM call_part_deletion").execute(&mut *conn).await.is_err());
	}

	#[test]
	fn a_part_is_its_week_s_day_then_p_and_its_number() {
		assert_eq!(split_key("2026-11-06"), Some(("2026-11-06", 1)));
		assert_eq!(split_key("2026-11-06-p2"), Some(("2026-11-06", 2)));
		assert_eq!(split_key("2026-11-06-p99"), Some(("2026-11-06", 99)));
		for key in ["2026-11-06-p1", "2026-11-06-p0", "2026-11-06-p02", "2026-11-06-p100", "2026-11-06-p", "2026-11-06-px"] {
			assert_eq!(split_key(key), None, "{key}");
		}
		assert_eq!(part_key("2026-11-06", 1), "2026-11-06");
		assert_eq!(part_key("2026-11-06", 3), "2026-11-06-p3");
	}

	#[tokio::test]
	async fn each_part_has_its_own_link_and_receipts_and_the_week_goes_once_every_part_has() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		keep(
			&mut conn,
			&Week {
				from: "2026-11-06",
				to: "2026-11-12",
				body: b"{}",
				taken_at: "2026-11-13T08:00:00.000Z",
			},
		)
		.await
		.unwrap();
		let (one, two) = ("aaaaaaaaaaa", "bbbbbbbbbbb");
		assert_eq!(record_link(&mut conn, "2026-11-06", one, "2026-11-14T10:00:00.000Z").await.unwrap(), Recorded::Added);
		assert_eq!(record_link(&mut conn, "2026-11-06-p2", two, "2026-11-14T10:01:00.000Z").await.unwrap(), Recorded::Added);
		assert_eq!(record_link(&mut conn, "2026-11-06-p1", two, "2026-11-14T10:01:00.000Z").await.unwrap(), Recorded::NoWeek);
		let links = links(&mut conn).await.unwrap();
		assert_eq!((links["2026-11-06"].youtube_id.as_str(), links["2026-11-06-p2"].youtube_id.as_str()), (one, two));

		let made = |key: &str, step, id: &str| Receipt {
			from: key.to_owned(),
			step,
			youtube_id: id.to_owned(),
			size: 1,
			deleted_at: "2026-11-14T12:00:00.000Z".to_owned(),
		};
		assert!(
			write_receipt(&mut conn, &made("2026-11-06-p2", Step::Video, one), 2).await.is_err(),
			"a part's receipt names its own part's link"
		);
		assert!(
			write_receipt(&mut conn, &made("2026-11-06-p2", Step::Video, two), 1).await.is_err(),
			"no part past its week's"
		);
		write_receipt(&mut conn, &made("2026-11-06-p2", Step::Video, two), 2).await.unwrap();
		assert_eq!(let_go(&mut conn).await.unwrap(), ["2026-11-06-p2"]);
		assert!(weeks_let_go(&mut conn).await.unwrap().is_empty(), "part 1 still has its video");
		assert_eq!(receipt(&mut conn, "2026-11-06", Step::Video).await.unwrap(), None);
		write_receipt(&mut conn, &made("2026-11-06", Step::Video, one), 2).await.unwrap();
		write_receipt(&mut conn, &made("2026-11-06-p2", Step::Rest, two), 2).await.unwrap();
		assert_eq!(weeks_let_go(&mut conn).await.unwrap(), ["2026-11-06"]);
		assert_eq!(
			receipts(&mut conn).await.unwrap(),
			[
				made("2026-11-06", Step::Video, one),
				made("2026-11-06-p2", Step::Video, two),
				made("2026-11-06-p2", Step::Rest, two)
			]
		);
	}

	#[tokio::test]
	async fn what_was_recorded_before_parts_is_part_one_and_the_old_tables_take_no_row() {
		// The migrations before parts, a week let go under them, then the rest.
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		let before = MIGRATOR.iter().filter(|migration| migration.version < 20_261_114_000_200).cloned().collect::<Vec<_>>();
		let early = sqlx::migrate::Migrator {
			migrations: std::borrow::Cow::Owned(before),
			..sqlx::migrate::Migrator::DEFAULT
		};
		early.run(&pool).await.unwrap();
		for sql in [
			"INSERT INTO call_week VALUES ('2026-10-30', '2026-11-05', printf('%064d', 0), 2, '{}', '2026-11-06T08:00:00.000Z')",
			"INSERT INTO call_video_link VALUES ('2026-10-30', 'dQw4w9WgXcQ', '2026-11-07T10:00:00.000Z')",
			"INSERT INTO call_video_deletion VALUES ('2026-10-30', 'video', 'dQw4w9WgXcQ', 95, '2026-11-07T12:00:00.000Z')",
			"INSERT INTO call_video_deletion VALUES ('2026-10-30', 'rest', 'dQw4w9WgXcQ', 17, '2026-11-07T12:01:00.000Z')",
		] {
			sqlx::query(sql).execute(&pool).await.unwrap();
		}
		MIGRATOR.run(&pool).await.unwrap();
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(links(&mut conn).await.unwrap()["2026-10-30"].youtube_id, ID);
		assert_eq!(
			receipts(&mut conn).await.unwrap().iter().map(|r| (r.from.as_str(), r.step, r.size)).collect::<Vec<_>>(),
			[("2026-10-30", Step::Video, 95), ("2026-10-30", Step::Rest, 17)]
		);
		assert_eq!(weeks_let_go(&mut conn).await.unwrap(), ["2026-10-30"]);
		let old: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM call_video_link").fetch_one(&mut *conn).await.unwrap();
		assert_eq!(old, 1, "the copy stays");
		assert!(sqlx::query("INSERT INTO call_video_link VALUES ('2026-10-30', 'aaaaaaaaaaa', 'x')")
			.execute(&mut *conn)
			.await
			.is_err());
		assert!(sqlx::query("INSERT INTO call_video_deletion VALUES ('2026-10-30', 'video', 'dQw4w9WgXcQ', 1, 'x')")
			.execute(&mut *conn)
			.await
			.is_err());
	}

	async fn receipt_of(conn: &mut SqliteConnection, step: Step) -> Option<Receipt> {
		receipt(conn, "2026-10-30", step).await.unwrap()
	}
}
