//! The owner putting chosen takes at the front of the transcriber's line
//! (paulgsc/server#1090, paulgsc/server#1091): `voice_note_fast_track`.
//!
//! `20261121000500_create_voice_note_fast_track.up.sql` records the design;
//! in short:
//!
//! - **The newest act decides.** [`act`] appends `fast` or `cancel`; a note
//!   is fast-tracked while its newest act is `fast`. The same act again
//!   writes nothing.
//! - **Only a take that still waits.** A note home has heard (or found
//!   silent) is not put first: nothing is left to hear.
//! - **Read by the queue.** `crate::transcript::waiting_first` puts a
//!   fast-tracked note first, and its [`crate::transcript::Urgency`] tells
//!   the transcriber not to give the lane its turn before it.
//! - **Per subject.** Every query takes the subject and filters on it.

use crate::{
	transcript::{self, Hearing},
	VoiceError,
};
use sqlx::SqliteConnection;

/// What the owner did: put a take first, or take that back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
	Fast,
	Cancel,
}

impl Act {
	/// The stored name.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Fast => "fast",
			Self::Cancel => "cancel",
		}
	}
}

/// What [`act`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acted {
	/// The act is the note's newest now.
	Recorded,
	/// The note's newest act already said so: nothing written.
	Unchanged,
	/// The subject holds no such note.
	NotHeld,
	/// Home has already heard it, or found it silent: nothing to put first.
	Ended,
}

/// Records `act` on `subject`'s note `note_id`, at `now`. Run it in the
/// transaction that holds the request's subject.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn act(conn: &mut SqliteConnection, subject: &str, note_id: &str, act: Act, now: &str) -> Result<Acted, VoiceError> {
	match transcript::heard(conn, subject, note_id).await? {
		Hearing::NotHeld => return Ok(Acted::NotHeld),
		Hearing::Heard(_) => return Ok(Acted::Ended),
		Hearing::Waiting => {}
	}
	let fast = is_fast(conn, subject, note_id).await?;
	if fast == (act == Act::Fast) {
		return Ok(Acted::Unchanged);
	}
	let name = act.as_str();
	sqlx::query!(
		"INSERT INTO voice_note_fast_track (subject_id, note_id, n, act, acted_at)
		 VALUES (?1, ?2, (SELECT COALESCE(MAX(n), 0) + 1 FROM voice_note_fast_track WHERE subject_id = ?1 AND note_id = ?2), ?3, ?4)",
		subject,
		note_id,
		name,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok(Acted::Recorded)
}

/// Whether `subject`'s note `note_id` is fast-tracked: its newest act is
/// `fast`.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn is_fast(conn: &mut SqliteConnection, subject: &str, note_id: &str) -> Result<bool, VoiceError> {
	Ok(
		sqlx::query_scalar!(
			r#"SELECT COALESCE((SELECT act = 'fast' FROM voice_note_fast_track WHERE subject_id = ?1 AND note_id = ?2 ORDER BY n DESC LIMIT 1), 0) AS "fast!: bool""#,
			subject,
			note_id
		)
		.fetch_one(&mut *conn)
		.await?,
	)
}

#[cfg(test)]
mod tests {
	use super::{act, is_fast, Act, Acted};
	use crate::{describe, put_note, Said, SaidBite};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";
	const B: &str = "subject-b";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	async fn note(pool: &SqlitePool, subject: &str, id: &str) {
		let said = Said {
			of: Some("studio"),
			of_ref: Some("intro:s1:1"),
			at: Some("2026-11-21T10:00:00.000Z"),
			content_type: Some("audio/webm"),
			bite: SaidBite::default(),
		};
		let mut tx = pool.begin().await.unwrap();
		put_note(&mut tx, subject, &describe(id, said, b"take").unwrap(), b"take", "2026-11-21T10:00:01.000Z")
			.await
			.unwrap();
		tx.commit().await.unwrap();
	}

	async fn acted(pool: &SqlitePool, subject: &str, id: &str, what: Act) -> Acted {
		let mut conn = pool.acquire().await.unwrap();
		act(&mut conn, subject, id, what, "now").await.unwrap()
	}

	async fn fast(pool: &SqlitePool, subject: &str, id: &str) -> bool {
		let mut conn = pool.acquire().await.unwrap();
		is_fast(&mut conn, subject, id).await.unwrap()
	}

	/// The newest act decides; the same act again writes nothing; another
	/// subject's note is not held; a heard take is not put first.
	#[tokio::test]
	async fn the_newest_act_decides_and_only_a_take_that_waits_is_put_first() {
		let pool = pool().await;
		note(&pool, A, "take").await;
		assert!(!fast(&pool, A, "take").await);
		assert_eq!(acted(&pool, A, "take", Act::Cancel).await, Acted::Unchanged, "nothing to take back");
		assert_eq!(acted(&pool, A, "take", Act::Fast).await, Acted::Recorded);
		assert!(fast(&pool, A, "take").await);
		assert_eq!(acted(&pool, A, "take", Act::Fast).await, Acted::Unchanged);
		assert_eq!(acted(&pool, B, "take", Act::Fast).await, Acted::NotHeld, "A's take is A's");
		assert_eq!(acted(&pool, A, "take", Act::Cancel).await, Acted::Recorded);
		assert!(!fast(&pool, A, "take").await);
		let acts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voice_note_fast_track").fetch_one(&pool).await.unwrap();
		assert_eq!(acts, 2, "append-only: one row per act that changed something");

		sqlx::query(
			"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, duration_ms, took_ms, heard_at)
			 VALUES (?1, 'take', 'm', 'e', 1, 'silent', 1, 1, 'now')",
		)
		.bind(A)
		.execute(&pool)
		.await
		.unwrap();
		assert_eq!(acted(&pool, A, "take", Act::Fast).await, Acted::Ended);
	}

	/// The table refuses an edit, and leaves with the account.
	#[tokio::test]
	async fn an_act_is_never_edited_and_leaves_with_the_account() {
		let pool = pool().await;
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, x'00')")
			.bind(A)
			.execute(&pool)
			.await
			.unwrap();
		note(&pool, A, "take").await;
		assert_eq!(acted(&pool, A, "take", Act::Fast).await, Acted::Recorded);
		assert!(sqlx::query("UPDATE voice_note_fast_track SET act = 'cancel'").execute(&pool).await.is_err());
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		let acts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voice_note_fast_track").fetch_one(&pool).await.unwrap();
		assert_eq!(acts, 0);
	}
}
