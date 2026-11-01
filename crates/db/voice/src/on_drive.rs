//! Home's word that Drive holds a Studio take too (paulgsc/some-ui#2401,
//! HF11): `studio_take_on_drive`.
//!
//! `20261101000800_create_studio_take_on_drive.up.sql` records the design;
//! in short:
//!
//! - **From the build-journal session's list, never from Drive.** Home has no
//!   Drive credential (the owner's route a). The session lists Drive's takes
//!   folder and puts the SHA-256 of each recording there in the Jobs mailbox;
//!   `file_host`'s poller keeps the list and, in the transaction that keeps
//!   it, calls [`record`], the one writer of this table.
//! - **Only takes home holds.** [`record`] matches the list against the
//!   subject's `studio` voice notes by their SHA-256 (`voice_note.sha256`)
//!   and records those alone; a hash naming nothing home holds is dropped,
//!   never kept.
//! - **Append-only.** A take is recorded once, by the first list that named
//!   it; nothing is updated (the trigger aborts an `UPDATE`) or deleted but
//!   with the account, and a later list that no longer names it takes
//!   nothing away.
//! - **Per subject.** Every query takes the subject and filters on it.
//! - **Read only by `GET /voice-notes/on-drive`** ([`on_drive`]), for the
//!   phone's own takes, by the phone's ids.

use sqlx::SqliteConnection;
use std::collections::HashSet;

/// The schema of a list of the takes Drive holds, as the mailbox takes it
/// (paulgsc/some-ui `apps/mailbox`, `brief`'s `drive-takes` kind).
pub const DRIVE_TAKES_SCHEMA: &str = "some-ui.drive-takes/1";

/// The most takes one read of [`on_drive`] answers about.
pub const MAX_IDS: usize = 100;

/// One list of the takes Drive holds: when the session looked, the mailbox
/// name it came under, and each recording's SHA-256.
#[derive(Debug, Clone, Copy)]
pub struct Listed<'a> {
	/// When the session listed Drive's takes folder (the list's `at`).
	pub at: &'a str,
	/// The list's name in the mailbox (`drive-takes-<stamp>.json`).
	pub name: &'a str,
	/// The SHA-256 of each recording, lower-case hex.
	pub sha256: &'a [String],
}

/// A take home has recorded as on Drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnDrive {
	/// The phone's id for the take, which names its voice note.
	pub note: String,
	/// When the session that first listed it there looked.
	pub listed_at: String,
}

/// Records each of `subject`'s `studio` voice notes whose SHA-256 `listed`
/// names, as of `now`; how many it recorded for the first time.
///
/// Run it in the transaction that keeps the list, so a list is kept and its
/// takes recorded together or not at all.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn record(conn: &mut SqliteConnection, subject: &str, listed: Listed<'_>, now: &str) -> Result<u64, sqlx::Error> {
	let named: HashSet<&str> = listed.sha256.iter().map(String::as_str).collect();
	if named.is_empty() {
		return Ok(0);
	}
	let takes = sqlx::query!(
		r#"SELECT note_id AS "note_id!: String", sha256 AS "sha256!: String"
		   FROM voice_note WHERE subject_id = ?1 AND of_kind = 'studio'"#,
		subject
	)
	.fetch_all(&mut *conn)
	.await?;
	let mut recorded = 0;
	for take in takes.iter().filter(|take| named.contains(take.sha256.as_str())) {
		recorded += sqlx::query!(
			"INSERT INTO studio_take_on_drive (subject_id, note_id, listed_at, list_name, kept_at)
			 VALUES (?1, ?2, ?3, ?4, ?5)
			 ON CONFLICT (subject_id, note_id) DO NOTHING",
			subject,
			take.note_id,
			listed.at,
			listed.name,
			now
		)
		.execute(&mut *conn)
		.await?
		.rows_affected();
	}
	Ok(recorded)
}

/// Which of `ids`, `subject`'s takes by the phone's ids, home has recorded
/// as on Drive, in the order asked; at most [`MAX_IDS`] are read.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn on_drive(conn: &mut SqliteConnection, subject: &str, ids: &[&str]) -> Result<Vec<OnDrive>, sqlx::Error> {
	let mut found = Vec::new();
	for id in ids.iter().take(MAX_IDS) {
		let row = sqlx::query!(
			r#"SELECT listed_at AS "listed_at!: String" FROM studio_take_on_drive WHERE subject_id = ?1 AND note_id = ?2"#,
			subject,
			id
		)
		.fetch_optional(&mut *conn)
		.await?;
		if let Some(row) = row {
			found.push(OnDrive {
				note: (*id).to_owned(),
				listed_at: row.listed_at,
			});
		}
	}
	Ok(found)
}

#[cfg(test)]
mod tests {
	use super::{on_drive, record, Listed};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";
	const B: &str = "subject-b";
	const NOW: &str = "2026-11-01T10:00:00.000Z";
	const AT: &str = "2026-11-01T09:15:00.000Z";
	const LIST: &str = "drive-takes-20261101T091500Z.json";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		for subject in [A, B] {
			sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?2)")
				.bind(subject)
				.bind(subject.as_bytes())
				.execute(&pool)
				.await
				.unwrap();
		}
		pool
	}

	async fn note(pool: &SqlitePool, subject: &str, id: &str, of: &str, sha256: &str) {
		sqlx::query(
			"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES (?1, ?2, ?3, 'p:s:t', 'then', 'audio/webm', 1, ?4, 'now')",
		)
		.bind(subject)
		.bind(id)
		.bind(of)
		.bind(sha256)
		.execute(pool)
		.await
		.unwrap();
	}

	fn hashes(names: &[&str]) -> Vec<String> {
		names.iter().map(|name| (*name).repeat(64)).collect()
	}

	async fn read(pool: &SqlitePool, subject: &str, ids: &[&str]) -> Vec<String> {
		let mut conn = pool.acquire().await.unwrap();
		on_drive(&mut conn, subject, ids).await.unwrap().into_iter().map(|take| take.note).collect()
	}

	#[tokio::test]
	async fn only_the_subjects_own_studio_takes_a_list_names_are_recorded_once() {
		let pool = pool().await;
		note(&pool, A, "t1", "studio", &"a".repeat(64)).await;
		note(&pool, A, "t2", "studio", &"b".repeat(64)).await;
		// The same bytes said for another kind, and another subject's take.
		note(&pool, A, "r1", "reflection", &"c".repeat(64)).await;
		note(&pool, B, "t9", "studio", &"a".repeat(64)).await;
		let sha256 = hashes(&["a", "c", "d"]);
		let mut conn = pool.acquire().await.unwrap();
		let listed = Listed {
			at: AT,
			name: LIST,
			sha256: &sha256,
		};
		assert_eq!(record(&mut conn, A, listed, NOW).await.unwrap(), 1);
		// A later list naming it again changes nothing.
		let later = Listed {
			at: "2026-11-02T09:00:00.000Z",
			name: "drive-takes-20261102T090000Z.json",
			sha256: &sha256,
		};
		assert_eq!(record(&mut conn, A, later, NOW).await.unwrap(), 0);
		drop(conn);
		assert_eq!(read(&pool, A, &["t2", "t1", "r1", "nope"]).await, vec!["t1"]);
		assert_eq!(read(&pool, B, &["t9", "t1"]).await, Vec::<String>::new());
		let mut conn = pool.acquire().await.unwrap();
		let first = on_drive(&mut conn, A, &["t1"]).await.unwrap();
		assert_eq!(first[0].listed_at, AT);
		// Nothing a list names that home does not hold is kept.
		let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM studio_take_on_drive").fetch_one(&mut *conn).await.unwrap();
		assert_eq!(rows, 1);
	}

	#[tokio::test]
	async fn no_row_is_ever_updated_and_rows_leave_with_the_account() {
		let pool = pool().await;
		note(&pool, A, "t1", "studio", &"a".repeat(64)).await;
		let sha256 = hashes(&["a"]);
		let mut conn = pool.acquire().await.unwrap();
		record(
			&mut conn,
			A,
			Listed {
				at: AT,
				name: LIST,
				sha256: &sha256,
			},
			NOW,
		)
		.await
		.unwrap();
		let updated = sqlx::query("UPDATE studio_take_on_drive SET listed_at = 'later'").execute(&mut *conn).await;
		assert!(updated.is_err());
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&mut *conn).await.unwrap();
		let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM studio_take_on_drive").fetch_one(&mut *conn).await.unwrap();
		assert_eq!(rows, 0);
	}

	#[tokio::test]
	async fn an_empty_list_records_nothing_and_a_read_stops_at_its_cap() {
		let pool = pool().await;
		note(&pool, A, "t1", "studio", &"a".repeat(64)).await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(record(&mut conn, A, Listed { at: AT, name: LIST, sha256: &[] }, NOW).await.unwrap(), 0);
		let sha256 = hashes(&["a"]);
		record(
			&mut conn,
			A,
			Listed {
				at: AT,
				name: LIST,
				sha256: &sha256,
			},
			NOW,
		)
		.await
		.unwrap();
		drop(conn);
		let mut ids: Vec<&str> = vec!["x"; super::MAX_IDS];
		ids.push("t1");
		assert_eq!(read(&pool, A, &ids).await, Vec::<String>::new());
	}
}
