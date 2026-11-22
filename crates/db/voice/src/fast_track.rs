//! The owner putting chosen takes at the front of the transcriber's line
//! (paulgsc/server#1090, paulgsc/server#1091): `voice_note_fast_track`.
//!
//! `20261121000500_create_voice_note_fast_track.up.sql` records the design;
//! in short:
//!
//! - **The newest act decides, and a mark is one try.** [`act`] appends
//!   `fast` or `cancel`; a marked note the queue's model has never tried is
//!   `FastTrack`, and once tried it is `Wanted` (TR1 holds literally). The
//!   same act again writes nothing. Any kind of note may be marked, a take
//!   most often.
//! - **Only a note that waits, as it stands.** A `fast` act on a note home
//!   has heard (or found silent) with any model, set aside, let go of, or
//!   spent every attempt on is refused: it would order nothing. A model
//!   bump can change that answer.
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
	/// For a `fast` act: home won't hear it as it stands (set aside, its
	/// recording let go, or every attempt it gets spent), so putting it
	/// first would order nothing. A model bump can change this.
	NotWaiting,
}

/// Records `act` on `subject`'s note `note_id`, at `now`.
///
/// `attempts` is the most attempts one model makes on a note (the
/// transcriber's `ATTEMPTS`), and `keys` the keys home hears with: whether a
/// note still waits is read under the key the queue waits on it under
/// (`transcript::STANDING_KEY`: its model's, or the fresh key a standing
/// request to hear it again names, paulgsc/some-ui#3339). Run it in the
/// transaction that holds the request's subject.
///
/// A `fast` act is one try at the head of the line (the migration's doc):
/// the queue reads it as [`transcript::Urgency::FastTrack`] while the
/// queue's key has never tried the note, and as `Wanted` once it has.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn act(conn: &mut SqliteConnection, subject: &str, note_id: &str, act: Act, keys: transcript::Keys<'_>, attempts: i64, now: &str) -> Result<Acted, VoiceError> {
	match transcript::heard(conn, subject, note_id).await? {
		Hearing::NotHeld => return Ok(Acted::NotHeld),
		Hearing::Heard(_) => return Ok(Acted::Ended),
		Hearing::Waiting => {}
	}
	let verbatim_kind = transcript::VERBATIM;
	let state = sqlx::query!(
		r#"SELECT
		     COALESCE((SELECT f.act = 'fast' FROM voice_note_fast_track f
		               WHERE f.subject_id = k.subject_id AND f.note_id = k.note_id ORDER BY f.n DESC LIMIT 1), 0) AS "marked!: bool",
		     (SELECT COUNT(*) FROM voice_transcript t
		      WHERE t.subject_id = k.subject_id AND t.note_id = k.note_id AND t.model = k.key) AS "tried!: i64",
		     (k.of_kind <> 'jobs' AND EXISTS (SELECT 1 FROM voice_note_set_aside s WHERE s.subject_id = k.subject_id AND s.note_id = k.note_id))
		       OR NOT EXISTS (SELECT 1 FROM voice_note_audio a WHERE a.subject_id = k.subject_id AND a.note_id = k.note_id) AS "out!: bool"
		   FROM (SELECT b.subject_id, b.note_id, b.of_kind,
		                COALESCE((SELECT r.hears_under FROM voice_note_attention r WHERE r.subject_id = b.subject_id AND r.note_id = b.note_id AND r.act = 'hear_again' AND r.asked_under = b.base ORDER BY r.n DESC LIMIT 1), b.base) AS key
		         FROM (SELECT n.subject_id, n.note_id, n.of_kind, CASE WHEN n.of_kind = ?5 THEN ?4 ELSE ?3 END AS base
		               FROM voice_note n WHERE n.subject_id = ?1 AND n.note_id = ?2) b) k"#,
		subject,
		note_id,
		keys.plain,
		keys.verbatim,
		verbatim_kind
	)
	.fetch_one(&mut *conn)
	.await?;
	// As it stands, and for a `fast` act only: every attempt the queue's key
	// gets on the note spent, it would order nothing. (A note any model has
	// heard is `Ended`, above.)
	if act == Act::Fast && (state.out || state.tried >= attempts) {
		return Ok(Acted::NotWaiting);
	}
	// The same act again changes nothing the queue reads, so writes nothing.
	let unchanged = match act {
		Act::Fast => state.marked,
		Act::Cancel => !state.marked,
	};
	if unchanged {
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
		let keys = crate::transcript::Keys {
			plain: "m",
			verbatim: "m+verbatim-1",
		};
		act(&mut conn, subject, id, what, keys, 3, "now").await.unwrap()
	}

	async fn acts(pool: &SqlitePool) -> i64 {
		sqlx::query_scalar("SELECT COUNT(*) FROM voice_note_fast_track").fetch_one(pool).await.unwrap()
	}

	/// The newest act decides and the same act again writes nothing; a mark
	/// on a tried note still records (the queue reads it as `Wanted`);
	/// another subject's note is not held; a heard note is not put first.
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
			Acted::Unchanged,
			"tried since: still marked (the queue reads Wanted now), so nothing to write"
		);
		assert_eq!(acted(&pool, A, "take", Act::Cancel).await, Acted::Recorded, "a tried mark is still taken back");
		assert_eq!(
			acted(&pool, A, "take", Act::Fast).await,
			Acted::Recorded,
			"marked again on a tried note: recorded, and the queue reads it as Wanted, never FastTrack"
		);
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
