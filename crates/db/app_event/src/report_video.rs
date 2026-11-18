//! Each week's engagement report video once it is on `YouTube`, and each
//! step of letting it go (paulgsc/some-ui ENG12, #2700).
//!
//! By the call's rules and with the call's types (`call_week_repo::video`),
//! as the essays' are (paulgsc/server#821): a link is kept once per week and
//! video, the newest standing; a week goes video first, then the rest, each
//! with one receipt kept for good. Every row is the subject's, as
//! `report_week` is. A week whose video was let go is never given to the
//! render worker again ([`let_go`]).
//!
//! The rules are `20261118000200_create_report_video.up.sql`'s; this module
//! is the one that writes `report_video_link` and `report_video_deletion`.

use call_week_repo::video::{Link, Receipt, Recorded, Step};
use sqlx::SqliteConnection;
use std::collections::BTreeMap;

/// Records `youtube_id` as the week's link, once; `NoWeek` for a week
/// `subject` holds no report of.
///
/// # Errors
/// A storage failure, or an id the table's checks refuse.
pub async fn record_link(conn: &mut SqliteConnection, subject: &str, week: &str, youtube_id: &str, now: &str) -> Result<Recorded, sqlx::Error> {
	if crate::report::body(conn, subject, week).await?.is_none() {
		return Ok(Recorded::NoWeek);
	}
	let done = sqlx::query!(
		"INSERT INTO report_video_link (subject_id, week, youtube_id, recorded_at) VALUES (?, ?, ?, ?)
		 ON CONFLICT (subject_id, week, youtube_id) DO NOTHING",
		subject,
		week,
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
pub async fn links(conn: &mut SqliteConnection, subject: &str) -> Result<BTreeMap<String, Link>, sqlx::Error> {
	let rows = sqlx::query!(
		r#"SELECT week AS "week!", youtube_id AS "youtube_id!", recorded_at AS "recorded_at!"
		   FROM report_video_link WHERE subject_id = ? ORDER BY week, recorded_at, youtube_id"#,
		subject
	)
	.fetch_all(&mut *conn)
	.await?;
	// Oldest first within each week, so the newest is the one left.
	Ok(
		rows
			.into_iter()
			.map(|row| {
				(
					row.week,
					Link {
						youtube_id: row.youtube_id,
						recorded_at: row.recorded_at,
					},
				)
			})
			.collect(),
	)
}

/// Every receipt, by week and step; each [`Receipt::from`] is the week.
///
/// # Errors
/// A storage failure, or a step the table holds that this does not know.
pub async fn receipts(conn: &mut SqliteConnection, subject: &str) -> Result<Vec<Receipt>, sqlx::Error> {
	let rows = sqlx::query!(
		r#"SELECT week AS "week!", step AS "step!", youtube_id AS "youtube_id!", size AS "size!: i64", deleted_at AS "deleted_at!"
		   FROM report_video_deletion WHERE subject_id = ? ORDER BY week, step DESC"#,
		subject
	)
	.fetch_all(&mut *conn)
	.await?;
	rows
		.into_iter()
		.map(|row| {
			Ok(Receipt {
				step: Step::parse(&row.step).ok_or_else(|| sqlx::Error::Protocol(String::from("a receipt names no step this knows")))?,
				from: row.week,
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
pub async fn receipt(conn: &mut SqliteConnection, subject: &str, week: &str, step: Step) -> Result<Option<Receipt>, sqlx::Error> {
	let name = step.as_str();
	let row = sqlx::query!(
		r#"SELECT youtube_id AS "youtube_id!", size AS "size!: i64", deleted_at AS "deleted_at!"
		   FROM report_video_deletion WHERE subject_id = ? AND week = ? AND step = ?"#,
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

/// Writes a step's receipt; its [`Receipt::from`] is the week.
///
/// # Errors
/// A storage failure, or what the table refuses: a second receipt for the
/// step, a link the week never had, or the rest before the video.
pub async fn write_receipt(conn: &mut SqliteConnection, subject: &str, receipt: &Receipt) -> Result<(), sqlx::Error> {
	let step = receipt.step.as_str();
	sqlx::query!(
		"INSERT INTO report_video_deletion (subject_id, week, step, youtube_id, size, deleted_at) VALUES (?, ?, ?, ?, ?, ?)",
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

/// The weeks whose video `subject` let go, oldest first: never given to the
/// render worker again.
///
/// # Errors
/// A storage failure.
pub async fn let_go(conn: &mut SqliteConnection, subject: &str) -> Result<Vec<String>, sqlx::Error> {
	sqlx::query_scalar!(
		r#"SELECT week AS "week!" FROM report_video_deletion WHERE subject_id = ? AND step = 'video' ORDER BY week"#,
		subject
	)
	.fetch_all(&mut *conn)
	.await
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::IsoWeek;
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";
	const B: &str = "subject-b";
	const NOW: &str = "2026-11-17T09:00:00.000Z";
	const ID: &str = "dQw4w9WgXcQ";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		for subject in [A, B] {
			sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?1)")
				.bind(subject)
				.execute(&pool)
				.await
				.unwrap();
		}
		let mut conn = pool.acquire().await.unwrap();
		crate::report::keep(&mut conn, A, IsoWeek::parse("2026-W46").unwrap(), "{}", NOW).await.unwrap();
		drop(conn);
		pool
	}

	fn made(step: Step) -> Receipt {
		Receipt {
			from: String::from("2026-W46"),
			step,
			youtube_id: String::from(ID),
			size: 10,
			deleted_at: String::from(NOW),
		}
	}

	#[tokio::test]
	async fn a_week_is_linked_once_and_goes_video_first_with_a_receipt_each_step() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(record_link(&mut conn, A, "2026-W47", ID, NOW).await.unwrap(), Recorded::NoWeek, "not held");
		assert_eq!(record_link(&mut conn, B, "2026-W46", ID, NOW).await.unwrap(), Recorded::NoWeek, "another subject's");
		assert!(write_receipt(&mut conn, A, &made(Step::Video)).await.is_err(), "no link yet");
		assert_eq!(record_link(&mut conn, A, "2026-W46", ID, NOW).await.unwrap(), Recorded::Added);
		assert_eq!(record_link(&mut conn, A, "2026-W46", ID, NOW).await.unwrap(), Recorded::Held);
		assert_eq!(links(&mut conn, A).await.unwrap()["2026-W46"].youtube_id, ID);
		assert!(links(&mut conn, B).await.unwrap().is_empty());
		assert!(write_receipt(&mut conn, A, &made(Step::Rest)).await.is_err(), "the rest only after the video");
		write_receipt(&mut conn, A, &made(Step::Video)).await.unwrap();
		assert!(write_receipt(&mut conn, A, &made(Step::Video)).await.is_err(), "once a step");
		write_receipt(&mut conn, A, &made(Step::Rest)).await.unwrap();
		assert_eq!(receipts(&mut conn, A).await.unwrap(), [made(Step::Video), made(Step::Rest)]);
		assert_eq!(receipt(&mut conn, A, "2026-W46", Step::Rest).await.unwrap(), Some(made(Step::Rest)));
		assert_eq!(let_go(&mut conn, A).await.unwrap(), ["2026-W46"]);
		assert!(let_go(&mut conn, B).await.unwrap().is_empty());
	}

	#[tokio::test]
	async fn links_and_receipts_are_never_rewritten_and_leave_only_with_their_account() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		record_link(&mut conn, A, "2026-W46", ID, NOW).await.unwrap();
		write_receipt(&mut conn, A, &made(Step::Video)).await.unwrap();
		for statement in [
			"UPDATE report_video_link SET recorded_at = 'x'",
			"UPDATE report_video_deletion SET size = 0",
			"DELETE FROM report_video_link",
			"DELETE FROM report_video_deletion",
		] {
			assert!(sqlx::query(statement).execute(&mut *conn).await.is_err(), "{statement}");
		}
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&mut *conn).await.unwrap();
		assert!(links(&mut conn, A).await.unwrap().is_empty() && receipts(&mut conn, A).await.unwrap().is_empty());
		assert!(crate::report::held(&mut conn, A).await.unwrap().is_empty(), "the week went too");
	}
}
