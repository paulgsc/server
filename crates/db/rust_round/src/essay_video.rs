//! Each week's Rust Rounds essay video once it is on `YouTube`, and each
//! step of letting it go (RR16, paulgsc/some-ui#2691).
//!
//! By the call's rules and with the call's types (`call_week_repo::video`):
//! a link is kept once per week and video, the newest standing; a week goes
//! video first, then the rest, each with one receipt kept for good. Unlike
//! the call's, every row is the subject's: the video plays the owner's own
//! takes (docs/identity.md invariant 23's one named exception).
//!
//! A week is also **withdrawn** when a take its essay plays is deleted from
//! My data ([`withdraw`]): both steps' receipts at once, `take-deleted`,
//! naming the note, linked or not. A week let go either way is never given
//! to the render worker again ([`let_go`]).
//!
//! The rules are `20261110000200_create_rust_essay_video.up.sql`'s; this
//! module is the one that writes `rust_essay_video_link` and
//! `rust_essay_video_deletion`.

use call_week_repo::video::{Link, Receipt, Recorded, Step};
use sqlx::SqliteConnection;
use std::collections::BTreeMap;

/// Records `youtube_id` as the essay week's link, once; `NoWeek` for a week
/// `subject` holds no essay of (`held`).
///
/// # Errors
/// A storage failure, or an id the table's checks refuse.
pub async fn record_link(conn: &mut SqliteConnection, subject: &str, week: &str, held: bool, youtube_id: &str, now: &str) -> Result<Recorded, sqlx::Error> {
	if !held || !crate::week::is_week_id(week) {
		return Ok(Recorded::NoWeek);
	}
	let done = sqlx::query!(
		"INSERT INTO rust_essay_video_link (subject_id, week_id, youtube_id, recorded_at) VALUES (?, ?, ?, ?)
		 ON CONFLICT (subject_id, week_id, youtube_id) DO NOTHING",
		subject,
		week,
		youtube_id,
		now,
	)
	.execute(&mut *conn)
	.await?;
	Ok(if done.rows_affected() == 1 { Recorded::Added } else { Recorded::Held })
}

/// Every essay week's link: the newest recorded of each, by week.
///
/// # Errors
/// A storage failure.
pub async fn links(conn: &mut SqliteConnection, subject: &str) -> Result<BTreeMap<String, Link>, sqlx::Error> {
	let rows = sqlx::query!(
		r#"SELECT week_id AS "week_id!", youtube_id AS "youtube_id!", recorded_at AS "recorded_at!"
		   FROM rust_essay_video_link WHERE subject_id = ? ORDER BY week_id, recorded_at, youtube_id"#,
		subject
	)
	.fetch_all(&mut *conn)
	.await?;
	Ok(
		rows
			.into_iter()
			.map(|row| {
				(
					row.week_id,
					Link {
						youtube_id: row.youtube_id,
						recorded_at: row.recorded_at,
					},
				)
			})
			.collect(),
	)
}

/// Every receipt of a week the owner let go, by week and step; each
/// [`Receipt::from`] is the week. A withdrawn week's are [`withdrawn`]'s.
///
/// # Errors
/// A storage failure, or a step the table holds that this does not know.
pub async fn receipts(conn: &mut SqliteConnection, subject: &str) -> Result<Vec<Receipt>, sqlx::Error> {
	let rows = sqlx::query!(
		r#"SELECT week_id AS "week_id!", step AS "step!", youtube_id AS "youtube_id!", size AS "size!: i64", deleted_at AS "deleted_at!"
		   FROM rust_essay_video_deletion WHERE subject_id = ? AND cause = 'let-go' ORDER BY week_id, step DESC"#,
		subject
	)
	.fetch_all(&mut *conn)
	.await?;
	rows
		.into_iter()
		.map(|row| {
			let step = Step::parse(&row.step).ok_or_else(|| sqlx::Error::Decode((String::from("unknown step ") + &row.step).into()))?;
			Ok(Receipt {
				from: row.week_id,
				step,
				youtube_id: row.youtube_id,
				size: row.size,
				deleted_at: row.deleted_at,
			})
		})
		.collect()
}

/// The receipt of a let-go week's step, if it was let go.
///
/// # Errors
/// A storage failure.
pub async fn receipt(conn: &mut SqliteConnection, subject: &str, week: &str, step: Step) -> Result<Option<Receipt>, sqlx::Error> {
	let name = step.as_str();
	let row = sqlx::query!(
		r#"SELECT youtube_id AS "youtube_id!", size AS "size!: i64", deleted_at AS "deleted_at!"
		   FROM rust_essay_video_deletion WHERE subject_id = ? AND week_id = ? AND step = ? AND cause = 'let-go'"#,
		subject,
		week,
		name,
	)
	.fetch_optional(&mut *conn)
	.await?;
	Ok(row.map(|row| Receipt {
		from: week.to_owned(),
		step,
		youtube_id: row.youtube_id,
		size: row.size,
		deleted_at: row.deleted_at,
	}))
}

/// Writes a let-go step's receipt; its [`Receipt::from`] is the week.
///
/// # Errors
/// A storage failure, or what the table refuses: a second receipt for the
/// step, a link the week never had, or the rest before the video.
pub async fn write_receipt(conn: &mut SqliteConnection, subject: &str, receipt: &Receipt) -> Result<(), sqlx::Error> {
	let step = receipt.step.as_str();
	sqlx::query!(
		"INSERT INTO rust_essay_video_deletion (subject_id, week_id, step, cause, youtube_id, note_id, size, deleted_at)
		 VALUES (?, ?, ?, 'let-go', ?, NULL, ?, ?)",
		subject,
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

/// A week withdrawn because a take it plays was deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Withdrawn {
	pub week: String,
	/// The deleted take's voice-note id.
	pub note: String,
	/// The bytes that went, video and rest together.
	pub size: i64,
	pub deleted_at: String,
}

/// Withdraws `week` for the deleted take `note`; whether it did.
///
/// Both steps' receipts, in the caller's transaction. A week already let go
/// or withdrawn keeps what it has.
///
/// # Errors
/// A storage failure.
pub async fn withdraw(conn: &mut SqliteConnection, subject: &str, week: &str, note: &str, size: i64, now: &str) -> Result<bool, sqlx::Error> {
	let had = sqlx::query_scalar!(
		r#"SELECT COUNT(*) AS "n!: i64" FROM rust_essay_video_deletion WHERE subject_id = ? AND week_id = ?"#,
		subject,
		week
	)
	.fetch_one(&mut *conn)
	.await?;
	if had > 0 {
		// Let go already: its video went, and with it the takes it played.
		return Ok(false);
	}
	let link = links(conn, subject).await?.remove(week).map(|link| link.youtube_id);
	for (step, bytes) in [("video", size), ("rest", 0)] {
		sqlx::query!(
			"INSERT INTO rust_essay_video_deletion (subject_id, week_id, step, cause, youtube_id, note_id, size, deleted_at)
			 VALUES (?, ?, ?, 'take-deleted', ?, ?, ?, ?)",
			subject,
			week,
			step,
			link,
			note,
			bytes,
			now,
		)
		.execute(&mut *conn)
		.await?;
	}
	Ok(true)
}

/// Every week withdrawn for a deleted take.
///
/// # Errors
/// A storage failure.
pub async fn withdrawn(conn: &mut SqliteConnection, subject: &str) -> Result<Vec<Withdrawn>, sqlx::Error> {
	let rows = sqlx::query!(
		r#"SELECT week_id AS "week_id!", note_id AS "note_id!", size AS "size!: i64", deleted_at AS "deleted_at!"
		   FROM rust_essay_video_deletion WHERE subject_id = ? AND cause = 'take-deleted' AND step = 'video' ORDER BY week_id"#,
		subject
	)
	.fetch_all(&mut *conn)
	.await?;
	Ok(
		rows
			.into_iter()
			.map(|row| Withdrawn {
				week: row.week_id,
				note: row.note_id,
				size: row.size,
				deleted_at: row.deleted_at,
			})
			.collect(),
	)
}

/// The weeks whose video went, let go or withdrawn: the render worker is
/// never given them again.
///
/// # Errors
/// A storage failure.
pub async fn let_go(conn: &mut SqliteConnection, subject: &str) -> Result<Vec<String>, sqlx::Error> {
	sqlx::query_scalar!(
		r#"SELECT week_id AS "week_id!" FROM rust_essay_video_deletion WHERE subject_id = ? AND step = 'video' ORDER BY week_id"#,
		subject
	)
	.fetch_all(&mut *conn)
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
		for subject in [ME, THEM] {
			sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?1)")
				.bind(subject)
				.execute(&pool)
				.await
				.unwrap();
		}
		pool
	}

	const ID: &str = "dQw4w9WgXcQ";
	const ME: &str = "me";
	const THEM: &str = "them";
	const WEEK: &str = "2026-w46";

	#[tokio::test]
	async fn an_essay_week_goes_video_first_then_the_rest_once_linked_each_kept_for_good_and_only_its_subjects() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(record_link(&mut conn, ME, WEEK, false, ID, "t").await.unwrap(), Recorded::NoWeek);
		assert_eq!(record_link(&mut conn, ME, "../x", true, ID, "t").await.unwrap(), Recorded::NoWeek);
		assert_eq!(record_link(&mut conn, ME, WEEK, true, ID, "2026-11-17T10:00:00.000Z").await.unwrap(), Recorded::Added);
		assert_eq!(record_link(&mut conn, ME, WEEK, true, ID, "2026-11-17T11:00:00.000Z").await.unwrap(), Recorded::Held);
		assert_eq!(links(&mut conn, ME).await.unwrap()[WEEK].youtube_id, ID);
		assert!(links(&mut conn, THEM).await.unwrap().is_empty(), "another subject's");

		let made = |step, size| Receipt {
			from: WEEK.to_owned(),
			step,
			youtube_id: ID.to_owned(),
			size,
			deleted_at: "2026-11-17T12:00:00.000Z".to_owned(),
		};
		assert!(write_receipt(&mut conn, THEM, &made(Step::Video, 9)).await.is_err(), "no link of theirs");
		assert!(write_receipt(&mut conn, ME, &made(Step::Rest, 7)).await.is_err(), "the rest goes only after the video");
		write_receipt(&mut conn, ME, &made(Step::Video, 90)).await.unwrap();
		assert!(write_receipt(&mut conn, ME, &made(Step::Video, 90)).await.is_err(), "one receipt a step");
		write_receipt(&mut conn, ME, &made(Step::Rest, 7)).await.unwrap();
		assert_eq!(receipts(&mut conn, ME).await.unwrap(), [made(Step::Video, 90), made(Step::Rest, 7)]);
		assert_eq!(receipt(&mut conn, ME, WEEK, Step::Video).await.unwrap(), Some(made(Step::Video, 90)));
		assert_eq!(let_go(&mut conn, ME).await.unwrap(), [WEEK]);
		assert!(!withdraw(&mut conn, ME, WEEK, "note-1", 5, "t").await.unwrap(), "let go already");
		for table in ["rust_essay_video_link", "rust_essay_video_deletion"] {
			let update = String::from("UPDATE ") + table + " SET youtube_id = 'bbbbbbbbbbb'";
			assert!(sqlx::query(&update).execute(&mut *conn).await.is_err(), "{table} UPDATE");
			let delete = String::from("DELETE FROM ") + table;
			assert!(sqlx::query(&delete).execute(&mut *conn).await.is_err(), "{table} DELETE");
		}

		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(ME).execute(&mut *conn).await.unwrap();
		assert!(
			links(&mut conn, ME).await.unwrap().is_empty() && let_go(&mut conn, ME).await.unwrap().is_empty(),
			"both leave with the account"
		);
	}

	#[tokio::test]
	async fn a_week_whose_take_is_deleted_is_withdrawn_both_steps_at_once_linked_or_not() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		assert!(withdraw(&mut conn, ME, WEEK, "note-1", 1234, "2026-11-20T00:00:00.000Z").await.unwrap());
		assert!(!withdraw(&mut conn, ME, WEEK, "note-2", 1, "t").await.unwrap(), "once");
		record_link(&mut conn, ME, "2026-w47", true, ID, "t").await.unwrap();
		assert!(withdraw(&mut conn, ME, "2026-w47", "note-3", 0, "t").await.unwrap());
		assert_eq!(let_go(&mut conn, ME).await.unwrap(), [WEEK, "2026-w47"]);
		assert!(receipts(&mut conn, ME).await.unwrap().is_empty(), "a withdrawal is no let-go receipt");
		let withdrawn = withdrawn(&mut conn, ME).await.unwrap();
		assert_eq!(
			withdrawn.iter().map(|w| (w.week.as_str(), w.note.as_str(), w.size)).collect::<Vec<_>>(),
			[(WEEK, "note-1", 1234), ("2026-w47", "note-3", 0)]
		);
		assert!(
			sqlx::query("INSERT INTO rust_essay_video_deletion VALUES ('me', '2026-w48', 'video', 'take-deleted', NULL, NULL, 0, 't')")
				.execute(&mut *conn)
				.await
				.is_err(),
			"a withdrawal names its take"
		);
	}
}
