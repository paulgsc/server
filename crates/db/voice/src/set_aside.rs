//! The owner's "Let go" on a voice note home holds (the owner's D3 on
//! paulgsc/some-ui epic #2368; paulgsc/some-ui#2372, MIC2): the take is set
//! aside and kept, never deleted. `voice_note_set_aside`.
//!
//! `20261101000900_create_voice_note_set_aside.up.sql` records the design;
//! in short:
//!
//! - **Its own table, so no note is updated.** A note is never rewritten
//!   (docs/identity.md invariant 23), so the mark sits beside it, keyed as
//!   the note is.
//! - **Only a note the subject holds.** [`set_aside`] marks a note in the
//!   statement that reads it, and marks nothing for a note the subject does
//!   not hold.
//! - **Append-only, and the first mark stands.** A note is marked once;
//!   nothing is updated (the trigger aborts an `UPDATE`) or deleted but with
//!   the account, so a repeat mark changes nothing, its time included. There
//!   is no un-set: the phone has no undo for Let go.
//! - **Nothing is let go of on home.** The note, its audio and any transcript
//!   stay. What a mark changes is what does work on the note: the
//!   transcriber's queue ([`crate::transcript::waiting`]) skips it, and so do
//!   the transcripts filed for the mailbox and the reflections index
//!   (`mailbox_repo`).
//! - **Per subject.** Every query takes the subject and filters on it.
//! - **Written only by `PUT /voice-notes/:id/set-aside`** (`file_host`'s
//!   `handlers::voice`).

use crate::{instant, VoiceError};
use sqlx::SqliteConnection;

/// What a mark did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetAside {
	/// The note is set aside now.
	Marked,
	/// The note was already set aside: nothing written, the first mark's
	/// time stands.
	AlreadyMarked,
	/// The subject holds no such note: nothing written.
	NotHeld,
}

/// `at` as home keeps a mark's time (ISO-8601 UTC to the millisecond), if
/// it is an RFC 3339 instant.
#[must_use]
pub fn when(at: &str) -> Option<String> {
	instant(at)
}

/// Sets `subject`'s note `note_id` aside as of `at` (as [`when`] gives it),
/// recorded by home at `now`.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the write and the read that
/// tells an earlier mark from no note see one snapshot.
///
/// # Errors
/// [`VoiceError::Storage`] for any `sqlx` failure; nothing is written then.
pub async fn set_aside(conn: &mut SqliteConnection, subject: &str, note_id: &str, at: &str, now: &str) -> Result<SetAside, VoiceError> {
	let marked = sqlx::query!(
		"INSERT INTO voice_note_set_aside (subject_id, note_id, set_aside_at, stored_at)
		 SELECT subject_id, note_id, ?3, ?4 FROM voice_note WHERE subject_id = ?1 AND note_id = ?2
		 ON CONFLICT (subject_id, note_id) DO NOTHING",
		subject,
		note_id,
		at,
		now
	)
	.execute(&mut *conn)
	.await?
	.rows_affected();
	if marked > 0 {
		return Ok(SetAside::Marked);
	}
	let held = sqlx::query_scalar!(
		r#"SELECT EXISTS (SELECT 1 FROM voice_note WHERE subject_id = ?1 AND note_id = ?2) AS "held!: bool""#,
		subject,
		note_id
	)
	.fetch_one(&mut *conn)
	.await?;
	Ok(if held { SetAside::AlreadyMarked } else { SetAside::NotHeld })
}

/// When `subject` set its note `note_id` aside, if it did.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn set_aside_at(conn: &mut SqliteConnection, subject: &str, note_id: &str) -> Result<Option<String>, VoiceError> {
	Ok(
		sqlx::query_scalar!("SELECT set_aside_at FROM voice_note_set_aside WHERE subject_id = ?1 AND note_id = ?2", subject, note_id)
			.fetch_optional(&mut *conn)
			.await?,
	)
}

#[cfg(test)]
mod tests {
	use super::{set_aside, set_aside_at, when, SetAside};
	use crate::transcript::waiting;
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";
	const B: &str = "subject-b";
	const AT: &str = "2026-11-01T09:15:00.000Z";
	const LATER: &str = "2026-11-02T09:15:00.000Z";
	const NOW: &str = "2026-11-01T09:15:01.000Z";

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

	async fn note(pool: &SqlitePool, subject: &str, id: &str, stored_at: &str) {
		sqlx::query(
			"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES (?1, ?2, 'reflection', 'session-1', 'then', 'audio/webm', 1, 's', ?3)",
		)
		.bind(subject)
		.bind(id)
		.bind(stored_at)
		.execute(pool)
		.await
		.unwrap();
		sqlx::query("INSERT INTO voice_note_audio (subject_id, note_id, body) VALUES (?1, ?2, x'00')")
			.bind(subject)
			.bind(id)
			.execute(pool)
			.await
			.unwrap();
	}

	async fn mark(pool: &SqlitePool, subject: &str, id: &str, at: &str) -> SetAside {
		let mut conn = pool.acquire().await.unwrap();
		set_aside(&mut conn, subject, id, at, NOW).await.unwrap()
	}

	async fn rows(pool: &SqlitePool, table: &str) -> i64 {
		sqlx::query_scalar(&(String::from("SELECT COUNT(*) FROM ") + table)).fetch_one(pool).await.unwrap()
	}

	#[test]
	fn a_marks_time_is_kept_as_utc_to_the_millisecond() {
		assert_eq!(when("2026-11-01T02:15:00-07:00").as_deref(), Some(AT));
		assert_eq!(when("soon"), None);
		assert_eq!(when(""), None);
	}

	#[tokio::test]
	async fn the_table_holds_exactly_its_described_columns() {
		let pool = pool().await;
		let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('voice_note_set_aside') ORDER BY cid")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(columns, ["subject_id", "note_id", "set_aside_at", "stored_at"]);
	}

	/// The first mark stands, a repeat changes nothing, and a note the
	/// subject does not hold, another subject's included, is never marked.
	/// The note and its audio stay.
	#[tokio::test]
	async fn a_held_note_is_set_aside_once_and_kept() {
		let pool = pool().await;
		note(&pool, A, "n1", "2026-10-31T10:00:00.000Z").await;
		note(&pool, B, "n2", "2026-10-31T10:00:00.000Z").await;

		assert_eq!(mark(&pool, A, "n1", AT).await, SetAside::Marked);
		assert_eq!(mark(&pool, A, "n1", LATER).await, SetAside::AlreadyMarked);
		assert_eq!(mark(&pool, A, "nope", AT).await, SetAside::NotHeld);
		assert_eq!(mark(&pool, A, "n2", AT).await, SetAside::NotHeld, "B's note is not A's to mark");

		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(set_aside_at(&mut conn, A, "n1").await.unwrap().as_deref(), Some(AT), "the first mark's time stands");
		assert_eq!(set_aside_at(&mut conn, B, "n2").await.unwrap(), None);
		drop(conn);
		assert_eq!(rows(&pool, "voice_note_set_aside").await, 1);
		assert_eq!(rows(&pool, "voice_note").await, 2, "nothing is let go of on home");
		assert_eq!(rows(&pool, "voice_note_audio").await, 2);
	}

	#[tokio::test]
	async fn no_mark_is_ever_updated_and_marks_leave_with_the_account() {
		let pool = pool().await;
		note(&pool, A, "n1", "2026-10-31T10:00:00.000Z").await;
		note(&pool, B, "n1", "2026-10-31T10:00:00.000Z").await;
		assert_eq!(mark(&pool, A, "n1", AT).await, SetAside::Marked);
		assert_eq!(mark(&pool, B, "n1", AT).await, SetAside::Marked);
		let refused = sqlx::query("UPDATE voice_note_set_aside SET set_aside_at = ?1").bind(LATER).execute(&pool).await;
		assert!(refused.unwrap_err().to_string().contains("append-only"));
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		let left: Vec<String> = sqlx::query_scalar("SELECT subject_id FROM voice_note_set_aside").fetch_all(&pool).await.unwrap();
		assert_eq!(left, [B]);
	}

	/// The transcriber's queue skips a note set aside before any model heard
	/// it, and keeps waiting on the others.
	#[tokio::test]
	async fn the_transcribers_queue_skips_a_note_set_aside() {
		const MODEL: &str = "ggml-small-q5_1.bin";
		let pool = pool().await;
		note(&pool, A, "kept", "2026-10-31T10:00:00.000Z").await;
		note(&pool, A, "let-go", "2026-10-31T10:01:00.000Z").await;
		note(&pool, B, "let-go", "2026-10-31T10:02:00.000Z").await;
		let queued = |pool: SqlitePool| async move {
			let mut conn = pool.acquire().await.unwrap();
			waiting(&mut conn, MODEL, 3, 10)
				.await
				.unwrap()
				.into_iter()
				.map(|note| (note.subject, note.note_id))
				.collect::<Vec<_>>()
		};
		assert_eq!(queued(pool.clone()).await.len(), 3);

		assert_eq!(mark(&pool, A, "let-go", AT).await, SetAside::Marked);
		assert_eq!(
			queued(pool.clone()).await,
			[(A.to_owned(), "kept".to_owned()), (B.to_owned(), "let-go".to_owned())],
			"only A's mark, on A's note"
		);
	}
}
