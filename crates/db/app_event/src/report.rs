//! Each week's engagement report, as home first wrote it (paulgsc/some-ui
//! ENG12, #2700).
//!
//! `report_week` (paulgsc/server#821) holds one row per subject and ISO
//! week: the [`weekly`](crate::weekly) read's JSON exactly as `file_host`'s
//! `report_weeks` worker gives it to the render worker.
//!
//! - **Once, as first written.** [`keep`] stores a week only while the
//!   subject holds none for it, and never replaces one: events that reach
//!   home after the week was written are in the next week's comparisons, not
//!   in this one's report, so a re-render reads the same bytes.
//! - **Append-only.** No row is updated or deleted while its account stands
//!   (the migration's triggers); it leaves with the account.
//! - **Per subject, and only per subject.** Every query takes the subject.
//!
//! `20261118000100_create_report_week.up.sql` records the design.

use crate::IsoWeek;
use sqlx::SqliteConnection;

/// What [`keep`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
	/// Home did not hold the week: it is kept now.
	Added,
	/// Home already held a body for the week: nothing written.
	Held,
	/// The subject has no account: nothing written.
	NoAccount,
}

/// Keeps `body` as `subject`'s report of `week`, unless the subject already
/// holds one or has no account.
///
/// One statement, so a deletion between the check and the write cannot
/// leave a row behind; the caller holds the deletion lock across it as well
/// (docs/identity.md invariant 9).
///
/// # Errors
/// A storage failure.
pub async fn keep(conn: &mut SqliteConnection, subject: &str, week: IsoWeek, body: &str, now: &str) -> Result<Kept, sqlx::Error> {
	let label = week.label();
	let added = sqlx::query!(
		"INSERT INTO report_week (subject_id, week, body, written_at)
		SELECT ?1, ?2, ?3, ?4 WHERE EXISTS (SELECT 1 FROM account WHERE subject_id = ?1)
		ON CONFLICT (subject_id, week) DO NOTHING",
		subject,
		label,
		body,
		now
	)
	.execute(&mut *conn)
	.await?
	.rows_affected();
	if added == 1 {
		return Ok(Kept::Added);
	}
	if held(conn, subject).await?.contains(&label) {
		Ok(Kept::Held)
	} else {
		Ok(Kept::NoAccount)
	}
}

/// Every week `subject` holds a report of, oldest first (`2026-W46`).
///
/// # Errors
/// A storage failure.
pub async fn held(conn: &mut SqliteConnection, subject: &str) -> Result<Vec<String>, sqlx::Error> {
	sqlx::query_scalar!(r#"SELECT week AS "week!: String" FROM report_week WHERE subject_id = ?1 ORDER BY week"#, subject)
		.fetch_all(&mut *conn)
		.await
}

/// `subject`'s report of `week`, as written, if home holds one.
///
/// # Errors
/// A storage failure.
pub async fn body(conn: &mut SqliteConnection, subject: &str, week: &str) -> Result<Option<String>, sqlx::Error> {
	sqlx::query_scalar!(r#"SELECT body AS "body!: String" FROM report_week WHERE subject_id = ?1 AND week = ?2"#, subject, week)
		.fetch_optional(&mut *conn)
		.await
}

#[cfg(test)]
mod tests {
	use super::{body, held, keep, Kept};
	use crate::IsoWeek;
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";
	const B: &str = "subject-b";
	const NOW: &str = "2026-11-16T09:00:00.000Z";

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
		pool
	}

	fn week(text: &str) -> IsoWeek {
		IsoWeek::parse(text).unwrap()
	}

	#[tokio::test]
	async fn a_week_is_kept_once_as_first_written_and_only_for_its_subject() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(keep(&mut conn, A, week("2026-W46"), r#"{"week":"2026-W46"}"#, NOW).await.unwrap(), Kept::Added);
		assert_eq!(keep(&mut conn, A, week("2026-W46"), r#"{"week":"later"}"#, NOW).await.unwrap(), Kept::Held);
		assert_eq!(body(&mut conn, A, "2026-W46").await.unwrap().as_deref(), Some(r#"{"week":"2026-W46"}"#));
		assert_eq!(body(&mut conn, B, "2026-W46").await.unwrap(), None, "another subject's week is not theirs");
		assert_eq!(keep(&mut conn, A, week("2026-W45"), "{}", NOW).await.unwrap(), Kept::Added);
		assert_eq!(held(&mut conn, A).await.unwrap(), ["2026-W45", "2026-W46"], "oldest first");
		assert!(held(&mut conn, B).await.unwrap().is_empty());
		assert_eq!(keep(&mut conn, "subject-gone", week("2026-W46"), "{}", NOW).await.unwrap(), Kept::NoAccount);
	}

	#[tokio::test]
	async fn a_kept_week_is_never_rewritten_and_leaves_only_with_its_account() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		keep(&mut conn, A, week("2026-W46"), "{}", NOW).await.unwrap();
		keep(&mut conn, B, week("2026-W46"), "{}", NOW).await.unwrap();
		assert!(sqlx::query("UPDATE report_week SET body = 'x'").execute(&mut *conn).await.is_err());
		assert!(sqlx::query("DELETE FROM report_week WHERE subject_id = ?1").bind(A).execute(&mut *conn).await.is_err());
		assert!(
			sqlx::query("INSERT INTO report_week (subject_id, week, body, written_at) VALUES (?1, '2026-w47', '{}', 'now')")
				.bind(A)
				.execute(&mut *conn)
				.await
				.is_err(),
			"a week is named as IsoWeek names it"
		);
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&mut *conn).await.unwrap();
		assert!(held(&mut conn, A).await.unwrap().is_empty(), "gone with the account");
		assert_eq!(held(&mut conn, B).await.unwrap(), ["2026-W46"], "nobody else's");
	}
}
