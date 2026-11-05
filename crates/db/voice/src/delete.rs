//! Deleting a voice note for good, from My data only (paulgsc/some-ui#2477,
//! MD3, under epic #2473), with a receipt in `voice_note_deletion` that goes
//! only with the account.
//!
//! The owner's rule (paulgsc/some-ui#2474, 2026-11-02): the tool can go,
//! never the work. A recording whose purpose is served elsewhere may be
//! deleted; what doing something produced (an answer, a spoken attempt)
//! never is. So:
//!
//! - **Two kinds of note can be deleted:** a Studio take
//!   ([`Deletable::StudioTake`]): a `studio` note whose `ref` names a take
//!   (`<project>:<shot>:<take>`) and that no idea's answer names; and an
//!   idea's spoken answer ([`Deletable::IdeaAnswer`]): a `studio` note whose
//!   `ref` is `idea:<idea>:<question>` or that `idea_answer` names. Every
//!   other kind is kept. The idea keeps a deleted answer's place, which then
//!   says the recording was deleted.
//! - **Each kind has its own switch.** [`SWITCHED_ON`] is the list the
//!   owner turns on, one kind at a time; what goes with each is recorded on
//!   paulgsc/some-ui#2265 and in that repo's `docs/discard-sites.md`.
//! - **What goes:** the note, its audio, what home heard in it (every
//!   transcript, its words and the take's vectors in the meaning index,
//!   which are made from them) and its set-aside mark, in one transaction.
//!   **What stays:** a copy of its words already filed for the Jobs mailbox
//!   (the mailbox keeps a name's body for good), the record that Drive
//!   holds it, the idea an answer belongs to (`idea_answer` still names the
//!   note, and the idea's read says when it was deleted), and the receipt.
//! - **The receipt** says what kind, when recorded and when it reached
//!   home, how many bytes, when deleted, and what went and what stayed, in
//!   fixed words. Never the words, the ref or the audio. It is never updated
//!   (the trigger aborts an `UPDATE`) and goes only with the account.
//! - **A repeat is answered from the receipt,** so a phone whose answer was
//!   lost learns the note is gone rather than that home never held it.
//! - **Per subject.** Every query takes the subject and filters on it.
//! - **Written only by `DELETE /voice-notes/:id`** (`file_host`'s
//!   `handlers::voice`), a paired phone's.
//! - **Read back by My data's list of what was deleted**
//!   (`GET /voice-notes/deleted`, paulgsc/server#667): [`receipts`], the
//!   newest [`RECEIPTS`] of the subject's, and how many in all.

use crate::{Of, VoiceError};
use serde::Serialize;
use sqlx::SqliteConnection;

/// A kind of note the delete machinery knows how to delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deletable {
	/// A Studio take: a `studio` note no idea's answer names. Its purpose is
	/// served once its video is published (paulgsc/some-ui#2495).
	StudioTake,
	/// An idea's spoken answer: a `studio` note an idea's answer names, or
	/// whose `ref` says it answers one.
	IdeaAnswer,
}

/// The kinds the owner has switched on (paulgsc/some-ui#2473).
pub const SWITCHED_ON: &[Deletable] = &[Deletable::StudioTake, Deletable::IdeaAnswer];

/// What went with a deleted note, in fixed words.
pub const RECORDING: &str = "recording";
/// What home heard in it.
pub const WORDS: &str = "words";
/// The owner's Let go mark on it.
pub const SET_ASIDE_MARK: &str = "set-aside mark";
/// Its words, filed for the Jobs mailbox.
pub const MAILBOX_COPY: &str = "mailbox copy";
/// The build-journal session's word that Drive holds it.
pub const DRIVE_COPY: &str = "drive copy";
/// The idea it answered, which keeps the answer's place.
pub const IDEA: &str = "idea";

/// What a delete left behind: the receipt, as home keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
	/// The note's id.
	pub id: String,
	/// What it was said for.
	pub of: String,
	/// When the phone recorded it.
	pub recorded_at: String,
	/// How many bytes its audio was.
	pub size: i64,
	/// When it reached home.
	pub stored_at: String,
	/// When home deleted it.
	pub deleted_at: String,
	/// What went, in fixed words.
	pub went: Vec<String>,
	/// What stayed, in fixed words.
	pub stayed: Vec<String>,
}

/// Why a note home holds was not deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
	/// Its kind can be deleted, but the owner has not switched it on.
	SwitchedOff,
	/// Its kind is kept for good: it is the work, or nothing does its job.
	ForGood,
}

/// What a delete did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Deleted {
	/// The note is gone, now or by an earlier delete, and this is its
	/// receipt.
	Gone(Receipt),
	/// Home holds the note and kept it: nothing written.
	Kept(Kept),
	/// The subject holds no such note, and deleted none: nothing written.
	NotHeld,
}

/// What a Studio idea's spoken answer's `ref` starts with
/// (`idea:<idea>:<question>`, paulgsc/some-ui MIC10); a take's is
/// `<project>:<shot>:<take>`.
const IDEA_ANSWER: &str = "idea:";

/// The kind a note is, as the machinery sees it, if it is one it can delete.
///
/// The note is said for `of_kind`, under `of_ref`; `answers` when an idea's
/// answer (`idea_answer`) names it. A `studio` note is an idea's spoken
/// answer when an answer names it or its `ref` says it answers one, and a
/// take otherwise; every other kind is kept.
#[must_use]
pub fn deletable(of_kind: &str, of_ref: &str, answers: bool) -> Option<Deletable> {
	(of_kind == Of::Studio.as_str()).then_some(if answers || of_ref.starts_with(IDEA_ANSWER) {
		Deletable::IdeaAnswer
	} else {
		Deletable::StudioTake
	})
}

/// The receipt home keeps for `subject`'s note `note_id`, if it deleted it.
///
/// # Errors
/// Any `sqlx` failure, or a stored list that does not read back.
pub async fn receipt(conn: &mut SqliteConnection, subject: &str, note_id: &str) -> Result<Option<Receipt>, VoiceError> {
	let Some(row) = sqlx::query!(
		"SELECT note_id, of_kind, recorded_at, size, stored_at, deleted_at, went, stayed
		 FROM voice_note_deletion WHERE subject_id = ?1 AND note_id = ?2",
		subject,
		note_id
	)
	.fetch_optional(&mut *conn)
	.await?
	else {
		return Ok(None);
	};
	let list = |text: &str| serde_json::from_str::<Vec<String>>(text).map_err(|_| VoiceError::Unreadable);
	Ok(Some(Receipt {
		id: row.note_id,
		of: row.of_kind,
		recorded_at: row.recorded_at,
		size: row.size,
		stored_at: row.stored_at,
		deleted_at: row.deleted_at,
		went: list(&row.went)?,
		stayed: list(&row.stayed)?,
	}))
}

/// The most receipts [`receipts`] answers: the newest, as My data lists them.
pub const RECEIPTS: i64 = 100;

/// `subject`'s receipts, newest deleted first (the note's id breaking a tie),
/// at most [`RECEIPTS`] of them, and how many it has in all.
///
/// # Errors
/// Any `sqlx` failure, or a stored list that does not read back.
pub async fn receipts(conn: &mut SqliteConnection, subject: &str) -> Result<(Vec<Receipt>, i64), VoiceError> {
	let total = sqlx::query_scalar!(r#"SELECT COUNT(*) AS "total!: i64" FROM voice_note_deletion WHERE subject_id = ?1"#, subject)
		.fetch_one(&mut *conn)
		.await?;
	let rows = sqlx::query!(
		"SELECT note_id, of_kind, recorded_at, size, stored_at, deleted_at, went, stayed
		 FROM voice_note_deletion WHERE subject_id = ?1
		 ORDER BY deleted_at DESC, note_id DESC
		 LIMIT ?2",
		subject,
		RECEIPTS
	)
	.fetch_all(&mut *conn)
	.await?;
	let list = |text: &str| serde_json::from_str::<Vec<String>>(text).map_err(|_| VoiceError::Unreadable);
	let receipts = rows
		.into_iter()
		.map(|row| {
			Ok(Receipt {
				went: list(&row.went)?,
				stayed: list(&row.stayed)?,
				id: row.note_id,
				of: row.of_kind,
				recorded_at: row.recorded_at,
				size: row.size,
				stored_at: row.stored_at,
				deleted_at: row.deleted_at,
			})
		})
		.collect::<Result<Vec<_>, VoiceError>>()?;
	Ok((receipts, total))
}

/// Deletes `subject`'s note `note_id` and what goes with it, and says, in
/// fixed words, what went and what stayed.
async fn remove(conn: &mut SqliteConnection, subject: &str, note_id: &str, answers: bool) -> Result<(Vec<String>, Vec<String>), VoiceError> {
	// Words went only if home heard some: a silent or failed attempt is no words.
	let heard = sqlx::query_scalar!(
		r#"SELECT EXISTS (SELECT 1 FROM voice_transcript WHERE subject_id = ?1 AND note_id = ?2 AND outcome = 'heard') AS "heard!: bool""#,
		subject,
		note_id
	)
	.fetch_one(&mut *conn)
	.await?;
	// The take's meaning (docs/identity.md invariant 35) is made from its
	// words, so it goes with them.
	sqlx::query!("DELETE FROM voice_note_meaning WHERE subject_id = ?1 AND note_id = ?2", subject, note_id)
		.execute(&mut *conn)
		.await?;
	sqlx::query!("DELETE FROM voice_transcript_word WHERE subject_id = ?1 AND note_id = ?2", subject, note_id)
		.execute(&mut *conn)
		.await?;
	sqlx::query!("DELETE FROM voice_transcript WHERE subject_id = ?1 AND note_id = ?2", subject, note_id)
		.execute(&mut *conn)
		.await?;
	let marked = sqlx::query!("DELETE FROM voice_note_set_aside WHERE subject_id = ?1 AND note_id = ?2", subject, note_id)
		.execute(&mut *conn)
		.await?
		.rows_affected();
	sqlx::query!("DELETE FROM voice_note_audio WHERE subject_id = ?1 AND note_id = ?2", subject, note_id)
		.execute(&mut *conn)
		.await?;
	sqlx::query!("DELETE FROM voice_note WHERE subject_id = ?1 AND note_id = ?2", subject, note_id)
		.execute(&mut *conn)
		.await?;
	let filed = sqlx::query_scalar!(
		r#"SELECT EXISTS (SELECT 1 FROM mailbox_transcript WHERE subject_id = ?1 AND note_id = ?2) AS "filed!: bool""#,
		subject,
		note_id
	)
	.fetch_one(&mut *conn)
	.await?;
	let on_drive = sqlx::query_scalar!(
		r#"SELECT EXISTS (SELECT 1 FROM studio_take_on_drive WHERE subject_id = ?1 AND note_id = ?2) AS "on_drive!: bool""#,
		subject,
		note_id
	)
	.fetch_one(&mut *conn)
	.await?;

	let mut went = vec![RECORDING.to_owned()];
	if heard {
		went.push(WORDS.to_owned());
	}
	if marked > 0 {
		went.push(SET_ASIDE_MARK.to_owned());
	}
	let mut stayed = Vec::new();
	if filed {
		stayed.push(MAILBOX_COPY.to_owned());
	}
	if on_drive {
		stayed.push(DRIVE_COPY.to_owned());
	}
	if answers {
		stayed.push(IDEA.to_owned());
	}
	Ok((went, stayed))
}

/// Deletes `subject`'s note `note_id` for good at `now`, if its kind is in
/// `on` (the handler passes [`SWITCHED_ON`]), and keeps its receipt.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the note read, its
/// deletion and the receipt see one snapshot and land together.
///
/// # Errors
/// [`VoiceError::Storage`] for any `sqlx` failure, and
/// [`VoiceError::Unreadable`] for a repeat whose receipt does not read back;
/// nothing is written then.
pub async fn delete(conn: &mut SqliteConnection, subject: &str, note_id: &str, on: &[Deletable], now: &str) -> Result<Deleted, VoiceError> {
	let Some(note) = sqlx::query!(
		"SELECT of_kind, of_ref, recorded_at, size, stored_at FROM voice_note WHERE subject_id = ?1 AND note_id = ?2",
		subject,
		note_id
	)
	.fetch_optional(&mut *conn)
	.await?
	else {
		return Ok(receipt(conn, subject, note_id).await?.map_or(Deleted::NotHeld, Deleted::Gone));
	};
	let answers = sqlx::query_scalar!(
		r#"SELECT EXISTS (SELECT 1 FROM idea_answer WHERE subject_id = ?1 AND note_id = ?2) AS "answers!: bool""#,
		subject,
		note_id
	)
	.fetch_one(&mut *conn)
	.await?;
	let Some(kind) = deletable(&note.of_kind, &note.of_ref, answers) else {
		return Ok(Deleted::Kept(Kept::ForGood));
	};
	if !on.contains(&kind) {
		return Ok(Deleted::Kept(Kept::SwitchedOff));
	}

	let (went, stayed) = remove(conn, subject, note_id, answers).await?;
	let receipt = Receipt {
		id: note_id.to_owned(),
		of: note.of_kind,
		recorded_at: note.recorded_at,
		size: note.size,
		stored_at: note.stored_at,
		deleted_at: now.to_owned(),
		went,
		stayed,
	};
	let went_json = serde_json::Value::from(receipt.went.clone()).to_string();
	let stayed_json = serde_json::Value::from(receipt.stayed.clone()).to_string();
	sqlx::query!(
		"INSERT INTO voice_note_deletion (subject_id, note_id, of_kind, recorded_at, size, stored_at, deleted_at, went, stayed)
		 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
		subject,
		receipt.id,
		receipt.of,
		receipt.recorded_at,
		receipt.size,
		receipt.stored_at,
		receipt.deleted_at,
		went_json,
		stayed_json
	)
	.execute(&mut *conn)
	.await?;
	Ok(Deleted::Gone(receipt))
}

#[cfg(test)]
mod tests {
	use super::{delete, receipt, receipts, Deletable, Deleted, Kept, DRIVE_COPY, IDEA, MAILBOX_COPY, RECEIPTS, RECORDING, SET_ASIDE_MARK, SWITCHED_ON, WORDS};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";
	const B: &str = "subject-b";
	const NOW: &str = "2026-11-03T09:00:00.000Z";
	const ON: &[Deletable] = &[Deletable::StudioTake];

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

	async fn run(pool: &SqlitePool, statement: &str, subject: &str, id: &str) {
		sqlx::query(statement).bind(subject).bind(id).execute(pool).await.unwrap();
	}

	async fn note(pool: &SqlitePool, subject: &str, id: &str, of: &str) {
		let insert = String::from(
			"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES (?1, ?2, '",
		) + of
			+ "', 'build-journal-ep04:s3:2', '2026-11-01T10:00:00.000Z', 'audio/webm', 120, 's', '2026-11-01T10:05:00.000Z')";
		run(pool, &insert, subject, id).await;
		run(pool, "INSERT INTO voice_note_audio (subject_id, note_id, body) VALUES (?1, ?2, x'00')", subject, id).await;
	}

	/// What home heard in it, a word of it, its Let go mark, its meaning,
	/// its words filed for the mailbox and Drive's listing of it.
	async fn everything_beside(pool: &SqlitePool, subject: &str, id: &str) {
		for statement in [
			"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, language, language_p, duration_ms, text, took_ms, heard_at)
			 VALUES (?1, ?2, 'm', 'e', 1, 'heard', 'ko', 0.9, 1, 'w', 1, 'now')",
			"INSERT INTO voice_transcript_word (subject_id, note_id, model, word_index, text, start_ms, end_ms) VALUES (?1, ?2, 'm', 0, 'w', 0, 1)",
			"INSERT INTO voice_note_set_aside (subject_id, note_id, set_aside_at, stored_at) VALUES (?1, ?2, 'then', 'now')",
			"INSERT INTO voice_note_meaning (subject_id, note_id, model, vector) VALUES (?1, ?2, 'm', zeroblob(1024))",
			"INSERT INTO mailbox_transcript (subject_id, note_id, model, n, filed_at) VALUES (?1, ?2, 'm', 1, 'now')",
			"INSERT INTO studio_take_on_drive (subject_id, note_id, listed_at, list_name, kept_at) VALUES (?1, ?2, 'then', 'drive-takes-20261101T000000Z.json', 'now')",
		] {
			run(pool, statement, subject, id).await;
		}
	}

	async fn rows(pool: &SqlitePool, table: &str, subject: &str, id: &str) -> i64 {
		sqlx::query_scalar(&(String::from("SELECT COUNT(*) FROM ") + table + " WHERE subject_id = ?1 AND note_id = ?2"))
			.bind(subject)
			.bind(id)
			.fetch_one(pool)
			.await
			.unwrap()
	}

	async fn gone(pool: &SqlitePool, subject: &str, id: &str, on: &[Deletable]) -> Deleted {
		let mut conn = pool.acquire().await.unwrap();
		delete(&mut conn, subject, id, on, NOW).await.unwrap()
	}

	#[tokio::test]
	async fn the_receipt_table_holds_exactly_its_described_columns() {
		let pool = pool().await;
		let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('voice_note_deletion') ORDER BY cid")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(
			columns,
			["subject_id", "note_id", "of_kind", "recorded_at", "size", "stored_at", "deleted_at", "went", "stayed"]
		);
	}

	/// Switched off, a take is kept whole and nothing is written, and so is
	/// an idea's spoken answer while only takes are on; every other kind is
	/// kept for good, switched on or not.
	#[tokio::test]
	async fn nothing_is_deleted_that_is_switched_off_or_kept_for_good() {
		let pool = pool().await;
		note(&pool, A, "take", "studio").await;
		note(&pool, A, "answer", "studio").await;
		run(
			&pool,
			"INSERT INTO idea_revision (subject_id, idea_id, revision, created_at, sent_at, title, topic, format, keywords, questions, stored_at)
			 VALUES (?1, ?2, 1, 'then', 'then', 't', 'Studio', 'short', '[]', 1, 'now')",
			A,
			"idea",
		)
		.await;
		run(
			&pool,
			"INSERT INTO idea_answer (subject_id, idea_id, revision, question, note_id, typed) VALUES (?1, 'idea', 1, 'what', ?2, NULL)",
			A,
			"answer",
		)
		.await;
		note(&pool, A, "reflection", "reflection").await;
		run(
			&pool,
			"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES (?1, ?2, 'studio', 'idea:i1:why', 'then', 'audio/webm', 1, 's', 'now')",
			A,
			"spoken",
		)
		.await;
		everything_beside(&pool, A, "take").await;

		assert_eq!(gone(&pool, A, "take", &[]).await, Deleted::Kept(Kept::SwitchedOff));
		assert_eq!(gone(&pool, A, "answer", ON).await, Deleted::Kept(Kept::SwitchedOff), "an answer has its own switch");
		assert_eq!(gone(&pool, A, "reflection", SWITCHED_ON).await, Deleted::Kept(Kept::ForGood));
		assert_eq!(gone(&pool, A, "spoken", ON).await, Deleted::Kept(Kept::SwitchedOff), "so does a spoken answer");
		for id in ["take", "answer", "reflection"] {
			assert_eq!(rows(&pool, "voice_note", A, id).await, 1, "{id}");
			assert_eq!(rows(&pool, "voice_note_audio", A, id).await, 1, "{id}");
		}
		for table in ["voice_transcript", "voice_transcript_word", "voice_note_set_aside", "voice_note_meaning"] {
			assert_eq!(rows(&pool, table, A, "take").await, 1, "{table}");
		}
		let receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voice_note_deletion").fetch_one(&pool).await.unwrap();
		assert_eq!(receipts, 0);
	}

	/// Switched on, a take goes with its audio, its words, its meaning and
	/// its mark (docs/identity.md invariant 35); its
	/// mailbox copy and Drive's listing stay; the receipt says so, and a
	/// repeat answers with the same receipt. Another subject's take is
	/// neither deleted nor named.
	#[tokio::test]
	async fn a_take_switched_on_goes_and_leaves_a_receipt() {
		let pool = pool().await;
		note(&pool, A, "take", "studio").await;
		note(&pool, B, "take", "studio").await;
		everything_beside(&pool, A, "take").await;

		assert_eq!(gone(&pool, B, "nope", ON).await, Deleted::NotHeld);
		let Deleted::Gone(first) = gone(&pool, A, "take", ON).await else {
			panic!("the take is gone");
		};
		assert_eq!(first.of, "studio");
		assert_eq!((first.recorded_at.as_str(), first.size, first.deleted_at.as_str()), ("2026-11-01T10:00:00.000Z", 120, NOW));
		assert_eq!(first.went, [RECORDING, WORDS, SET_ASIDE_MARK]);
		assert_eq!(first.stayed, [MAILBOX_COPY, DRIVE_COPY]);
		for table in [
			"voice_note",
			"voice_note_audio",
			"voice_transcript",
			"voice_transcript_word",
			"voice_note_set_aside",
			"voice_note_meaning",
		] {
			assert_eq!(rows(&pool, table, A, "take").await, 0, "{table}");
		}
		for table in ["mailbox_transcript", "studio_take_on_drive"] {
			assert_eq!(rows(&pool, table, A, "take").await, 1, "{table}");
		}
		assert_eq!(gone(&pool, A, "take", ON).await, Deleted::Gone(first.clone()), "a repeat reads the receipt");
		assert_eq!(rows(&pool, "voice_note", B, "take").await, 1, "B's take is B's");
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(receipt(&mut conn, B, "take").await.unwrap(), None);
	}

	/// A take home heard nothing in goes alone: its receipt names only the
	/// recording, a failed attempt to hear it not being words.
	#[tokio::test]
	async fn a_bare_takes_receipt_names_only_the_recording() {
		let pool = pool().await;
		note(&pool, A, "take", "studio").await;
		run(
			&pool,
			"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, took_ms, heard_at)
			 VALUES (?1, ?2, 'm', 'e', 1, 'failed', 1, 'now')",
			A,
			"take",
		)
		.await;
		let Deleted::Gone(receipt) = gone(&pool, A, "take", ON).await else {
			panic!("the take is gone");
		};
		assert_eq!(receipt.went, [RECORDING]);
		assert!(receipt.stayed.is_empty());
	}

	#[tokio::test]
	async fn a_receipt_is_never_rewritten_and_leaves_with_the_account() {
		let pool = pool().await;
		note(&pool, A, "take", "studio").await;
		note(&pool, B, "take", "studio").await;
		assert!(matches!(gone(&pool, A, "take", ON).await, Deleted::Gone(_)));
		assert!(matches!(gone(&pool, B, "take", ON).await, Deleted::Gone(_)));
		let refused = sqlx::query("UPDATE voice_note_deletion SET went = '[]'").execute(&pool).await;
		assert!(refused.unwrap_err().to_string().contains("append-only"));
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		let left: Vec<String> = sqlx::query_scalar("SELECT subject_id FROM voice_note_deletion").fetch_all(&pool).await.unwrap();
		assert_eq!(left, [B]);
	}
	/// Switched on, an idea's spoken answer goes as a take does, and the
	/// idea keeps its place: `idea_answer` still names the note, and the
	/// receipt says the idea stayed. A spoken answer no idea names goes
	/// with no idea to stay.
	#[tokio::test]
	async fn an_idea_answer_switched_on_goes_and_its_idea_stays() {
		let pool = pool().await;
		note(&pool, A, "answer", "studio").await;
		everything_beside(&pool, A, "answer").await;
		run(
			&pool,
			"INSERT INTO idea_revision (subject_id, idea_id, revision, created_at, sent_at, title, topic, format, keywords, questions, stored_at)
			 VALUES (?1, ?2, 1, 'then', 'then', 't', 'Studio', 'short', '[]', 1, 'now')",
			A,
			"idea",
		)
		.await;
		run(
			&pool,
			"INSERT INTO idea_answer (subject_id, idea_id, revision, question, note_id, typed) VALUES (?1, 'idea', 1, 'what', ?2, NULL)",
			A,
			"answer",
		)
		.await;
		run(
			&pool,
			"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES (?1, ?2, 'studio', 'idea:i1:why', 'then', 'audio/webm', 1, 's', 'now')",
			A,
			"spoken",
		)
		.await;

		let Deleted::Gone(answer) = gone(&pool, A, "answer", SWITCHED_ON).await else {
			panic!("the answer is gone");
		};
		assert_eq!(answer.went, [RECORDING, WORDS, SET_ASIDE_MARK]);
		assert_eq!(answer.stayed, [MAILBOX_COPY, DRIVE_COPY, IDEA]);
		for table in ["voice_note", "voice_note_audio", "voice_transcript", "voice_transcript_word", "voice_note_set_aside"] {
			assert_eq!(rows(&pool, table, A, "answer").await, 0, "{table}");
		}
		assert_eq!(rows(&pool, "idea_answer", A, "answer").await, 1, "the idea keeps its answer's place");
		let Deleted::Gone(spoken) = gone(&pool, A, "spoken", SWITCHED_ON).await else {
			panic!("the spoken answer is gone");
		};
		assert_eq!((spoken.went, spoken.stayed), (vec![RECORDING.to_owned()], Vec::new()));
	}

	/// The list of receipts is the subject's own, newest deleted first, at
	/// most `RECEIPTS`, with how many there are in all.
	#[tokio::test]
	async fn the_receipts_read_back_newest_first_bounded_and_the_subjects_own() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(receipts(&mut conn, A).await.unwrap(), (Vec::new(), 0));
		drop(conn);
		let many = usize::try_from(RECEIPTS).unwrap() + 2;
		// `n` as its last `width` digits, zero-padded.
		let digits = |n: usize, width: usize| -> String {
			let text = (n + 10_usize.pow(u32::try_from(width).unwrap())).to_string();
			text.chars().skip(1).collect()
		};
		let id_of = |n: usize| String::from("t") + &digits(n, 3);
		for n in 0..many {
			let id = id_of(n);
			let at = String::from("2026-11-03T09:") + &digits(n / 60, 2) + ":" + &digits(n % 60, 2) + ".000Z";
			note(&pool, A, &id, "studio").await;
			let mut conn = pool.acquire().await.unwrap();
			assert!(matches!(delete(&mut conn, A, &id, ON, &at).await.unwrap(), Deleted::Gone(_)));
		}
		note(&pool, B, "theirs", "studio").await;
		assert!(matches!(gone(&pool, B, "theirs", ON).await, Deleted::Gone(_)));

		let mut conn = pool.acquire().await.unwrap();
		let (listed, total) = receipts(&mut conn, A).await.unwrap();
		assert_eq!(total, i64::try_from(many).unwrap());
		assert_eq!(listed.len(), usize::try_from(RECEIPTS).unwrap());
		assert_eq!(listed.first().unwrap().id, id_of(many - 1), "the newest first");
		assert!(listed.iter().zip(listed.iter().skip(1)).all(|(newer, older)| newer.deleted_at >= older.deleted_at));
		assert!(listed.iter().all(|each| each.id != "theirs" && each.went == [RECORDING]));
		let (theirs, one) = receipts(&mut conn, B).await.unwrap();
		assert_eq!((theirs.len(), one), (1, 1));
	}
}
