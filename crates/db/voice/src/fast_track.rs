//! The owner putting chosen takes at the front of the transcriber's line
//! (paulgsc/server#1090, paulgsc/server#1091): `voice_note_fast_track`.
//!
//! `20261121000500_create_voice_note_fast_track.up.sql` records the design;
//! in short:
//!
//! - **The newest act decides, and a mark is one try.** [`act`] appends
//!   `fast` or `cancel`; a note's mark is live while its newest act is
//!   `fast` and home has made no attempt on it since. The same act again
//!   writes nothing. Any kind of note may be marked, a take most often.
//! - **Only a note that still waits.** A note home has heard (or found
//!   silent), set aside, let go of, or spent every attempt on is not put
//!   first: nothing it could do.
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
	/// Home is not going to hear it as it stands (set aside, its recording
	/// let go, or every attempt it gets spent): putting it first would do
	/// nothing.
	NotWaiting,
}

/// Records `act` on `subject`'s note `note_id`, at `now`.
///
/// `attempts` is the most attempts one model makes on a note (the
/// transcriber's `ATTEMPTS`). Run it in the transaction that holds the
/// request's subject.
///
/// A `fast` act is one try (the migration's doc): it records how many
/// attempts home has made on the note so far, and the queue treats the mark
/// as live only until home makes another.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn act(conn: &mut SqliteConnection, subject: &str, note_id: &str, act: Act, attempts: i64, now: &str) -> Result<Acted, VoiceError> {
	match transcript::heard(conn, subject, note_id).await? {
		Hearing::NotHeld => return Ok(Acted::NotHeld),
		Hearing::Heard(_) => return Ok(Acted::Ended),
		Hearing::Waiting => {}
	}
	let state = sqlx::query!(
		r#"SELECT
		     COALESCE((SELECT f.act = 'fast' AND f.attempts = (SELECT COUNT(*) FROM voice_transcript t WHERE t.subject_id = ?1 AND t.note_id = ?2)
		               FROM voice_note_fast_track f WHERE f.subject_id = ?1 AND f.note_id = ?2 ORDER BY f.n DESC LIMIT 1), 0) AS "live!: bool",
		     COALESCE((SELECT f.act = 'fast' FROM voice_note_fast_track f
		               WHERE f.subject_id = ?1 AND f.note_id = ?2 ORDER BY f.n DESC LIMIT 1), 0) AS "marked!: bool",
		     (SELECT COUNT(*) FROM voice_transcript t WHERE t.subject_id = ?1 AND t.note_id = ?2) AS "tried!: i64",
		     COALESCE((SELECT MAX(per) FROM (SELECT COUNT(*) AS per FROM voice_transcript t
		                                     WHERE t.subject_id = ?1 AND t.note_id = ?2 GROUP BY t.model)), 0) AS "most!: i64",
		     (n.of_kind <> 'jobs' AND EXISTS (SELECT 1 FROM voice_note_set_aside s WHERE s.subject_id = ?1 AND s.note_id = ?2))
		       OR NOT EXISTS (SELECT 1 FROM voice_note_audio a WHERE a.subject_id = ?1 AND a.note_id = ?2) AS "out!: bool"
		   FROM voice_note n WHERE n.subject_id = ?1 AND n.note_id = ?2"#,
		subject,
		note_id
	)
	.fetch_one(&mut *conn)
	.await?;
	// Home keeps no model key here: a model that has spent every attempt on
	// the note is taken as the queue's, so a new model hearing every note
	// again may refuse a mark it would have heard; the owner marks it again
	// once that model has tried it.
	if act == Act::Fast && (state.out || state.most >= attempts) {
		return Ok(Acted::NotWaiting);
	}
	// A live mark again is no change; a cancel clears a mark spent or not,
	// since a spent one still puts the note ahead of those no one waits on.
	let unchanged = match act {
		Act::Fast => state.live,
		Act::Cancel => !state.marked,
	};
	if unchanged {
		return Ok(Acted::Unchanged);
	}
	let name = act.as_str();
	sqlx::query!(
		"INSERT INTO voice_note_fast_track (subject_id, note_id, n, act, attempts, acted_at)
		 VALUES (?1, ?2, (SELECT COALESCE(MAX(n), 0) + 1 FROM voice_note_fast_track WHERE subject_id = ?1 AND note_id = ?2), ?3, ?4, ?5)",
		subject,
		note_id,
		name,
		state.tried,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok(Acted::Recorded)
}

#[cfg(test)]
mod tests {
	use super::{act, Act, Acted};
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

	async fn attempt(pool: &SqlitePool, id: &str, model: &str, outcome: &str) {
		sqlx::query(
			"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, duration_ms, took_ms, heard_at)
			 VALUES (?1, ?2, ?3, 'e', (SELECT COUNT(*) + 1 FROM voice_transcript WHERE subject_id = ?1 AND note_id = ?2 AND model = ?3), ?4, 1, 1, 'now')",
		)
		.bind(A)
		.bind(id)
		.bind(model)
		.bind(outcome)
		.execute(pool)
		.await
		.unwrap();
	}

	async fn acted(pool: &SqlitePool, subject: &str, id: &str, what: Act) -> Acted {
		let mut conn = pool.acquire().await.unwrap();
		act(&mut conn, subject, id, what, 3, "now").await.unwrap()
	}

	async fn acts(pool: &SqlitePool) -> i64 {
		sqlx::query_scalar("SELECT COUNT(*) FROM voice_note_fast_track").fetch_one(pool).await.unwrap()
	}

	/// The newest act decides and the same act again writes nothing; a mark
	/// is spent by the attempt that follows it, and marking again gives a new
	/// try; another subject's note is not held; a heard note is not put first.
	#[tokio::test]
	async fn a_mark_is_one_try_and_only_a_note_that_waits_is_put_first() {
		let pool = pool().await;
		note(&pool, A, "take").await;
		assert_eq!(acted(&pool, A, "take", Act::Cancel).await, Acted::Unchanged, "nothing to take back");
		assert_eq!(acted(&pool, A, "take", Act::Fast).await, Acted::Recorded);
		assert_eq!(acted(&pool, A, "take", Act::Fast).await, Acted::Unchanged);
		assert_eq!(acted(&pool, B, "take", Act::Fast).await, Acted::NotHeld, "A's take is A's");
		assert_eq!(acted(&pool, A, "take", Act::Cancel).await, Acted::Recorded);
		assert_eq!(acted(&pool, A, "take", Act::Fast).await, Acted::Recorded);
		attempt(&pool, "take", "m", "deadline").await;
		assert_eq!(
			acted(&pool, A, "take", Act::Fast).await,
			Acted::Recorded,
			"tried since: the mark is spent, and marked again it is a new try"
		);
		attempt(&pool, "take", "m", "deadline").await;
		assert_eq!(acted(&pool, A, "take", Act::Cancel).await, Acted::Recorded, "a spent mark is still taken back");
		assert_eq!(acts(&pool).await, 5, "append-only: one row per act that changed something");
		attempt(&pool, "take", "m", "silent").await;
		assert_eq!(acted(&pool, A, "take", Act::Fast).await, Acted::Ended);
	}

	/// A note home will not hear as it stands is not put first: every
	/// attempt one model makes spent, set aside, or its recording let go.
	#[tokio::test]
	async fn a_note_home_will_not_hear_is_not_put_first() {
		let pool = pool().await;
		for id in ["spent", "aside", "let-go"] {
			note(&pool, A, id).await;
		}
		for _ in 0..3 {
			attempt(&pool, "spent", "m", "deadline").await;
		}
		sqlx::query("INSERT INTO voice_note_set_aside (subject_id, note_id, set_aside_at, stored_at) VALUES (?1, 'aside', 'then', 'now')")
			.bind(A)
			.execute(&pool)
			.await
			.unwrap();
		sqlx::query("DELETE FROM voice_note_audio WHERE subject_id = ?1 AND note_id = 'let-go'")
			.bind(A)
			.execute(&pool)
			.await
			.unwrap();
		for id in ["spent", "aside", "let-go"] {
			assert_eq!(acted(&pool, A, id, Act::Fast).await, Acted::NotWaiting, "{id}");
		}
		assert_eq!(acts(&pool).await, 0);
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
		assert_eq!(acts(&pool).await, 0);
	}
}
