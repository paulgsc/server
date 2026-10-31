//! What home heard in each voice note (paulgsc/server#462): `voice_transcript`.
//!
//! `20261025000200_create_voice_transcript.up.sql` records the design; in
//! short:
//!
//! - **One row per attempt.** An attempt to hear one note with one model ends
//!   [`Outcome::Heard`] (with text), [`Outcome::Silent`] (no speech found), or
//!   as a failure naming its kind. [`keep_attempt`] writes it; nothing else
//!   writes this table.
//! - **Append-only.** No row is updated (the trigger aborts an `UPDATE`) or
//!   deleted but with the account. A model that has heard a note, or found
//!   it silent, never hears it again (a partial unique index holds that), and
//!   a better model adds rows of its own beside the old ones.
//! - **Bounded retries.** [`waiting`] names only the notes a model has not
//!   ended and has failed on fewer than `attempts` times, so one note whose
//!   audio cannot be heard is never retried forever.
//! - **Per subject.** A row is written only for a note the subject holds, in
//!   the statement that writes it, so nothing outlives its note or account.
//! - **Each heard word keeps its time** (paulgsc/some-ui#2085, GWB11;
//!   `20261030000300_create_voice_transcript_word.up.sql`). A heard attempt's
//!   [`Word`]s go to `voice_transcript_word` in the transaction that writes
//!   its row, append-only too, and [`words`] reads back those of the newest
//!   transcript that heard something, for the captions burned into a short.

use crate::VoiceError;
use sqlx::SqliteConnection;

/// One word heard, when it was said, and how sure the model was of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
	/// The word as heard, with any punctuation that clings to it. Never
	/// logged.
	pub text: String,
	/// When it began, in milliseconds from the note's start.
	pub start_ms: i64,
	/// When it ended, the same way; never before it began.
	pub end_ms: i64,
	/// How sure the model was of the word, 0 to 1: the lowest of its tokens'
	/// probabilities, so one unsure piece makes the word unsure
	/// (paulgsc/server#577, ST22). None for a word heard before it was kept
	/// (`20261031000200`), or one the model gave no number for.
	pub p: Option<f64>,
	/// Whisper's code for the language it was heard in (`en`, `ko`): home
	/// hears each piece of speech in its own (paulgsc/server#578, ST23). None
	/// for a word heard before it was kept (`20261031000300`).
	pub language: Option<String>,
}

/// How an attempt to hear a note ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
	/// Speech was found and heard.
	Heard {
		/// Whisper's code for the language detected (`en`, `ko`).
		language: String,
		/// That detection's probability, 0 to 1.
		language_p: f64,
		/// What was heard. Never logged.
		text: String,
		/// The words of `text` in the order said, each with its time. Never
		/// logged.
		words: Vec<Word>,
	},
	/// No speech was found: nothing was said.
	Silent,
	/// The audio did not decode.
	Unreadable,
	/// The audio is longer than the worker hears.
	TooLong,
	/// The attempt ran past its deadline.
	Deadline,
	/// The engine failed.
	Failed,
}

impl Outcome {
	/// The stored name, which is also the only thing about an attempt a log
	/// line or a metric may carry.
	#[must_use]
	pub const fn kind(&self) -> &'static str {
		match self {
			Self::Heard { .. } => "heard",
			Self::Silent => "silent",
			Self::Unreadable => "unreadable",
			Self::TooLong => "too_long",
			Self::Deadline => "deadline",
			Self::Failed => "failed",
		}
	}

	/// Every kind, in [`Self::kind`]'s order.
	pub const KINDS: [&'static str; 6] = ["heard", "silent", "unreadable", "too_long", "deadline", "failed"];
}

/// One attempt to hear a note, as [`keep_attempt`] stores it.
#[derive(Debug, Clone, PartialEq)]
pub struct Attempt<'a> {
	/// The model file it was heard with (`ggml-small-q5_1.bin`).
	pub model: &'a str,
	/// What ran it (`whisper.cpp 1.8.3`).
	pub engine: &'a str,
	pub outcome: Outcome,
	/// The audio's length, when it decoded.
	pub duration_ms: Option<i64>,
	/// How long the attempt took.
	pub took_ms: i64,
}

/// A note waiting to be heard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiting {
	pub subject: String,
	pub note_id: String,
}

/// The notes `model` has not ended (heard, or found silent) and has failed on
/// fewer than `attempts` times, oldest first, at most `limit`.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn waiting(conn: &mut SqliteConnection, model: &str, attempts: i64, limit: i64) -> Result<Vec<Waiting>, VoiceError> {
	let rows = sqlx::query!(
		r#"SELECT n.subject_id AS "subject_id!", n.note_id AS "note_id!" FROM voice_note n
		 WHERE NOT EXISTS (SELECT 1 FROM voice_transcript t
		                   WHERE t.subject_id = n.subject_id AND t.note_id = n.note_id AND t.model = ?1
		                     AND t.outcome IN ('heard', 'silent'))
		   AND (SELECT COUNT(*) FROM voice_transcript t
		        WHERE t.subject_id = n.subject_id AND t.note_id = n.note_id AND t.model = ?1) < ?2
		 ORDER BY n.stored_at, n.subject_id, n.note_id
		 LIMIT ?3"#,
		model,
		attempts,
		limit
	)
	.fetch_all(&mut *conn)
	.await?;
	Ok(
		rows
			.into_iter()
			.map(|row| Waiting {
				subject: row.subject_id,
				note_id: row.note_id,
			})
			.collect(),
	)
}

/// The audio `subject` holds under `note_id`, if any.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn audio(conn: &mut SqliteConnection, subject: &str, note_id: &str) -> Result<Option<Vec<u8>>, VoiceError> {
	Ok(
		sqlx::query_scalar!("SELECT body FROM voice_note_audio WHERE subject_id = ?1 AND note_id = ?2", subject, note_id)
			.fetch_optional(&mut *conn)
			.await?,
	)
}

/// What a kept attempt did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
	/// Stored as this model's attempt number `n` on the note.
	Added(i64),
	/// The model had already heard the note, or found it silent: nothing
	/// written.
	Ended,
	/// The subject holds no such note (it went with its account): nothing
	/// written.
	NoNote,
}

/// Stores `attempt` on `subject`'s note `note_id` as of `now`.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the reads that decide and
/// the write see one snapshot.
///
/// # Errors
/// [`VoiceError::Storage`] for any `sqlx` failure; nothing is written then.
pub async fn keep_attempt(conn: &mut SqliteConnection, subject: &str, note_id: &str, attempt: &Attempt<'_>, now: &str) -> Result<Kept, VoiceError> {
	let held = sqlx::query_scalar!(
		r#"SELECT EXISTS (SELECT 1 FROM voice_note WHERE subject_id = ?1 AND note_id = ?2) AS "held!: bool""#,
		subject,
		note_id
	)
	.fetch_one(&mut *conn)
	.await?;
	if !held {
		return Ok(Kept::NoNote);
	}
	let standing = sqlx::query!(
		r#"SELECT COUNT(*) AS "tried!: i64",
		          COALESCE(SUM(outcome IN ('heard', 'silent')), 0) AS "ended!: i64"
		   FROM voice_transcript WHERE subject_id = ?1 AND note_id = ?2 AND model = ?3"#,
		subject,
		note_id,
		attempt.model
	)
	.fetch_one(&mut *conn)
	.await?;
	if standing.ended > 0 {
		return Ok(Kept::Ended);
	}
	let n = standing.tried + 1;
	let kind = attempt.outcome.kind();
	let (language, language_p, text) = match &attempt.outcome {
		Outcome::Heard { language, language_p, text, .. } => (Some(language.as_str()), Some(*language_p), Some(text.as_str())),
		_ => (None, None, None),
	};
	sqlx::query!(
		"INSERT INTO voice_transcript
		   (subject_id, note_id, model, engine, attempt, outcome, language, language_p, duration_ms, text, took_ms, heard_at)
		 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
		subject,
		note_id,
		attempt.model,
		attempt.engine,
		n,
		kind,
		language,
		language_p,
		attempt.duration_ms,
		text,
		attempt.took_ms,
		now
	)
	.execute(&mut *conn)
	.await?;
	if let Outcome::Heard { words, .. } = &attempt.outcome {
		keep_words(conn, subject, note_id, attempt.model, words).await?;
	}
	Ok(Kept::Added(n))
}

/// Stores `words`, what `model` heard in `subject`'s note `note_id`, in the
/// transaction [`keep_attempt`] writes the transcript in.
///
/// A time the schema would refuse is held to it here (never negative, never
/// ending before it began) rather than refused, and so is how sure the model
/// was (not a number: none; past 0 or 1: held to it): a refused write fails
/// the whole attempt, and the note would wait to be heard again, failing the
/// same way, every pass.
async fn keep_words(conn: &mut SqliteConnection, subject: &str, note_id: &str, model: &str, words: &[Word]) -> Result<(), VoiceError> {
	for (index, word) in (0_i64..).zip(words) {
		let start_ms = word.start_ms.max(0);
		let end_ms = word.end_ms.max(start_ms);
		let p = word.p.filter(|p| p.is_finite()).map(|p| p.clamp(0.0, 1.0));
		// A language the schema would refuse is held to none, as the times are.
		let language = word
			.language
			.as_deref()
			.filter(|code| (2..=3).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_lowercase()));
		sqlx::query!(
			"INSERT INTO voice_transcript_word (subject_id, note_id, model, word_index, text, start_ms, end_ms, p, language)
			 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
			subject,
			note_id,
			model,
			index,
			word.text,
			start_ms,
			end_ms,
			p,
			language
		)
		.execute(&mut *conn)
		.await?;
	}
	Ok(())
}

/// The words of `subject`'s note `note_id`, each with its time, in the order
/// said.
///
/// They are those of the newest transcript that heard something, as a reader
/// of the text takes it, so the words always match that text. Empty when no
/// transcript heard the note, or when the newest was heard before words were
/// kept (`20261030000300`).
///
/// # Errors
/// Any `sqlx` failure.
pub async fn words(conn: &mut SqliteConnection, subject: &str, note_id: &str) -> Result<Vec<Word>, VoiceError> {
	Ok(
		sqlx::query_as!(
			Word,
			r#"SELECT w.text, w.start_ms, w.end_ms, w.p, w.language FROM voice_transcript_word w
			 WHERE w.subject_id = ?1 AND w.note_id = ?2
			   AND w.model = (SELECT t.model FROM voice_transcript t
			                  WHERE t.subject_id = ?1 AND t.note_id = ?2 AND t.outcome = 'heard'
			                  ORDER BY t.heard_at DESC, t.model DESC LIMIT 1)
			 ORDER BY w.word_index"#,
			subject,
			note_id
		)
		.fetch_all(&mut *conn)
		.await?,
	)
}

/// The words `model` heard in `subject`'s note `note_id`, in the order said.
///
/// Each with its time and how sure the model was: what a transcript filed
/// under that model carries (paulgsc/server#577). Empty for a transcript
/// heard before words were kept.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn words_heard_by(conn: &mut SqliteConnection, subject: &str, note_id: &str, model: &str) -> Result<Vec<Word>, sqlx::Error> {
	sqlx::query_as!(
		Word,
		r#"SELECT text, start_ms, end_ms, p, language FROM voice_transcript_word
		 WHERE subject_id = ?1 AND note_id = ?2 AND model = ?3
		 ORDER BY word_index"#,
		subject,
		note_id,
		model
	)
	.fetch_all(&mut *conn)
	.await
}

#[cfg(test)]
mod tests {
	use super::{audio, keep_attempt, waiting, words, words_heard_by, Attempt, Kept, Outcome, Waiting, Word};
	use crate::{describe, put_note, Said, SaidBite};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const PREVIOUS_MIGRATION: i64 = 20_261_025_000_100;
	const A: &str = "subject-a";
	const B: &str = "subject-b";
	const SMALL: &str = "ggml-small-q5_1.bin";
	const MEDIUM: &str = "ggml-medium-q5_0.bin";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	/// `subject`'s note `id`, stored at `stored_at`, its audio `body`.
	async fn note(pool: &SqlitePool, subject: &str, id: &str, stored_at: &str, body: &[u8]) {
		let said = Said {
			of: Some("reflection"),
			of_ref: Some("session-7f2a"),
			at: Some("2026-10-25T03:10:00Z"),
			content_type: Some("audio/webm"),
			bite: SaidBite::default(),
		};
		let mut tx = pool.begin().await.unwrap();
		put_note(&mut tx, subject, &describe(id, said, body).unwrap(), body, stored_at).await.unwrap();
		tx.commit().await.unwrap();
	}

	/// `text` heard, each word half a second long, one after another.
	fn heard(text: &str) -> Outcome {
		Outcome::Heard {
			language: String::from("en"),
			language_p: 0.99,
			text: text.to_owned(),
			words: said(text),
		}
	}

	fn said(text: &str) -> Vec<Word> {
		(0_i64..)
			.zip(text.split_whitespace())
			.map(|(n, word)| Word {
				text: word.to_owned(),
				start_ms: 500 * n,
				end_ms: 500 * n + 400,
				p: Some(0.9),
				language: Some(String::from("en")),
			})
			.collect()
	}

	async fn words_of(pool: &SqlitePool, subject: &str, id: &str) -> Vec<Word> {
		let mut conn = pool.acquire().await.unwrap();
		words(&mut conn, subject, id).await.unwrap()
	}

	fn attempt(model: &str, outcome: Outcome) -> Attempt<'_> {
		let duration_ms = if matches!(outcome, Outcome::Unreadable) { None } else { Some(28_400) };
		Attempt {
			model,
			engine: "whisper.cpp 1.8.3",
			outcome,
			duration_ms,
			took_ms: 21_000,
		}
	}

	async fn keep(pool: &SqlitePool, subject: &str, id: &str, attempt: &Attempt<'_>) -> Kept {
		let mut tx = pool.begin().await.unwrap();
		let kept = keep_attempt(&mut tx, subject, id, attempt, "2026-10-25T16:00:00.000Z").await.unwrap();
		tx.commit().await.unwrap();
		kept
	}

	async fn waiting_for(pool: &SqlitePool, model: &str) -> Vec<(String, String)> {
		let mut conn = pool.acquire().await.unwrap();
		waiting(&mut conn, model, 3, 20)
			.await
			.unwrap()
			.into_iter()
			.map(|Waiting { subject, note_id }| (subject, note_id))
			.collect()
	}

	fn named(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
		pairs.iter().map(|(subject, id)| ((*subject).to_owned(), (*id).to_owned())).collect()
	}

	#[tokio::test]
	async fn the_migration_round_trips() {
		let pool = pool().await;
		let table = || sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE name = 'voice_transcript'");
		assert_eq!(table().fetch_one(&pool).await.unwrap(), 1);
		MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
		assert_eq!(table().fetch_one(&pool).await.unwrap(), 0, "down drops it");
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES ('s', x'00')")
			.execute(&pool)
			.await
			.unwrap();
		sqlx::query("DELETE FROM account").execute(&pool).await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		assert_eq!(table().fetch_one(&pool).await.unwrap(), 1, "up recreates it");
	}

	/// The columns the migration describes and no others: a new one is a
	/// decision about what the server keeps from a note.
	#[tokio::test]
	async fn the_table_holds_exactly_its_described_columns() {
		let pool = pool().await;
		let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('voice_transcript') ORDER BY cid")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(
			columns,
			[
				"subject_id",
				"note_id",
				"model",
				"engine",
				"attempt",
				"outcome",
				"language",
				"language_p",
				"duration_ms",
				"text",
				"took_ms",
				"heard_at"
			]
		);
	}

	/// Oldest first, each model on its own, and a note a model has heard is
	/// never handed to it again.
	#[tokio::test]
	async fn notes_wait_oldest_first_until_heard() {
		let pool = pool().await;
		note(&pool, A, "late", "2026-10-25T03:00:00.000Z", b"two").await;
		note(&pool, B, "early", "2026-10-25T02:00:00.000Z", b"one").await;
		note(&pool, A, "first", "2026-10-25T01:00:00.000Z", b"zero").await;
		assert_eq!(waiting_for(&pool, SMALL).await, named(&[(A, "first"), (B, "early"), (A, "late")]));

		assert_eq!(keep(&pool, A, "first", &attempt(SMALL, heard("hello"))).await, Kept::Added(1));
		assert_eq!(keep(&pool, B, "early", &attempt(SMALL, Outcome::Silent)).await, Kept::Added(1));
		assert_eq!(waiting_for(&pool, SMALL).await, named(&[(A, "late")]), "heard and silent both end a note");
		assert_eq!(
			waiting_for(&pool, MEDIUM).await,
			named(&[(A, "first"), (B, "early"), (A, "late")]),
			"a better model hears every note again"
		);

		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(audio(&mut conn, A, "late").await.unwrap().as_deref(), Some(&b"two"[..]));
		assert_eq!(audio(&mut conn, B, "late").await.unwrap(), None, "B's ids are B's");
	}

	/// A Studio take is heard as any note is: it waits until heard, by each
	/// model on its own.
	#[tokio::test]
	async fn a_studio_take_waits_to_be_heard_like_any_note() {
		let pool = pool().await;
		let said = Said {
			of: Some("studio"),
			of_ref: Some("build-journal-ep04:s3:2"),
			at: Some("2026-10-30T18:20:00Z"),
			content_type: Some("audio/mp4"),
			bite: SaidBite::default(),
		};
		let mut tx = pool.begin().await.unwrap();
		put_note(&mut tx, A, &describe("t1", said, b"take").unwrap(), b"take", "2026-10-30T18:21:00.000Z")
			.await
			.unwrap();
		tx.commit().await.unwrap();
		assert_eq!(waiting_for(&pool, SMALL).await, named(&[(A, "t1")]));
		assert_eq!(keep(&pool, A, "t1", &attempt(SMALL, heard("shot three, take two"))).await, Kept::Added(1));
		assert_eq!(waiting_for(&pool, SMALL).await, named(&[]), "heard once");
		assert_eq!(waiting_for(&pool, MEDIUM).await, named(&[(A, "t1")]), "a better model hears it again");
	}

	/// A failure is kept and retried, at most `attempts` times; a note's
	/// ending is kept once, and nothing is written after it.
	#[tokio::test]
	async fn a_failing_note_stops_waiting_and_an_ended_one_takes_nothing_more() {
		let pool = pool().await;
		note(&pool, A, "bad", "2026-10-25T01:00:00.000Z", b"not audio").await;
		note(&pool, A, "good", "2026-10-25T02:00:00.000Z", b"audio").await;
		for (n, outcome) in [(1, Outcome::Unreadable), (2, Outcome::Deadline), (3, Outcome::Failed)] {
			assert_eq!(waiting_for(&pool, SMALL).await[0], named(&[(A, "bad")])[0], "attempt {n}");
			assert_eq!(keep(&pool, A, "bad", &attempt(SMALL, outcome)).await, Kept::Added(n));
		}
		assert_eq!(waiting_for(&pool, SMALL).await, named(&[(A, "good")]), "three failures and it stops waiting");

		assert_eq!(keep(&pool, A, "good", &attempt(SMALL, Outcome::TooLong)).await, Kept::Added(1));
		assert_eq!(keep(&pool, A, "good", &attempt(SMALL, heard("hello"))).await, Kept::Added(2));
		assert_eq!(keep(&pool, A, "good", &attempt(SMALL, heard("hello again"))).await, Kept::Ended, "a replay adds nothing");
		assert_eq!(keep(&pool, A, "good", &attempt(SMALL, Outcome::Silent)).await, Kept::Ended);
		assert_eq!(keep(&pool, A, "gone", &attempt(SMALL, heard("x"))).await, Kept::NoNote);

		let rows: Vec<(String, i64, String, Option<String>)> = sqlx::query_as("SELECT note_id, attempt, outcome, text FROM voice_transcript ORDER BY note_id, attempt")
			.fetch_all(&pool)
			.await
			.unwrap();
		let expected = [
			("bad", 1, "unreadable", None),
			("bad", 2, "deadline", None),
			("bad", 3, "failed", None),
			("good", 1, "too_long", None),
			("good", 2, "heard", Some("hello")),
		];
		let expected: Vec<(String, i64, String, Option<String>)> = expected
			.into_iter()
			.map(|(id, n, kind, text)| (id.to_owned(), n, kind.to_owned(), text.map(str::to_owned)))
			.collect();
		assert_eq!(rows, expected);
	}

	/// The schema holds the rules on its own, whatever writes to it.
	#[tokio::test]
	async fn the_schema_refuses_a_rewrite_a_second_ending_or_a_heard_row_without_text() {
		let pool = pool().await;
		note(&pool, A, "n1", "2026-10-25T01:00:00.000Z", b"audio").await;
		keep(&pool, A, "n1", &attempt(SMALL, heard("hello"))).await;
		let refused = sqlx::query("UPDATE voice_transcript SET text = 'other'").execute(&pool).await;
		assert!(refused.unwrap_err().to_string().contains("append-only"), "no row is ever updated");
		for insert in [
			// A second ending for the same note and model.
			"INSERT INTO voice_transcript VALUES ('subject-a', 'n1', 'ggml-small-q5_1.bin', 'e', 2, 'silent', NULL, NULL, 1, NULL, 1, 'now')",
			// Heard with no text.
			"INSERT INTO voice_transcript VALUES ('subject-a', 'n1', 'm', 'e', 1, 'heard', 'en', 0.9, 1, NULL, 1, 'now')",
			// Text on a failure.
			"INSERT INTO voice_transcript VALUES ('subject-a', 'n1', 'm', 'e', 1, 'failed', NULL, NULL, NULL, 'x', 1, 'now')",
			// An outcome the table does not know.
			"INSERT INTO voice_transcript VALUES ('subject-a', 'n1', 'm', 'e', 1, 'guessed', NULL, NULL, NULL, NULL, 1, 'now')",
		] {
			assert!(sqlx::query(insert).execute(&pool).await.is_err(), "{insert}");
		}
	}

	#[tokio::test]
	async fn the_transcripts_leave_with_the_account() {
		let pool = pool().await;
		for subject in [A, B] {
			sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?2)")
				.bind(subject)
				.bind(subject.as_bytes())
				.execute(&pool)
				.await
				.unwrap();
			note(&pool, subject, "n1", "2026-10-25T01:00:00.000Z", b"audio").await;
			keep(&pool, subject, "n1", &attempt(SMALL, heard("hello"))).await;
		}
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		let left: Vec<String> = sqlx::query_scalar("SELECT subject_id FROM voice_transcript").fetch_all(&pool).await.unwrap();
		assert_eq!(left, [B], "A's transcripts went with A's account, B's stayed");
	}

	#[tokio::test]
	async fn the_word_migration_round_trips() {
		const BEFORE_WORDS: i64 = 20_261_030_000_200;
		let pool = pool().await;
		let table = || sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'voice_transcript_word%'");
		assert_eq!(table().fetch_one(&pool).await.unwrap(), 3, "the table and its two triggers");
		MIGRATOR.undo(&pool, BEFORE_WORDS).await.unwrap();
		assert_eq!(table().fetch_one(&pool).await.unwrap(), 0, "down drops them");
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES ('s', x'00')")
			.execute(&pool)
			.await
			.unwrap();
		sqlx::query("DELETE FROM account").execute(&pool).await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		assert_eq!(table().fetch_one(&pool).await.unwrap(), 3, "up recreates them");
	}

	/// The columns the migration describes and no others: a new one is a
	/// decision about what the server keeps from a note.
	#[tokio::test]
	async fn the_word_table_holds_exactly_its_described_columns() {
		let pool = pool().await;
		let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('voice_transcript_word') ORDER BY cid")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(columns, ["subject_id", "note_id", "model", "word_index", "text", "start_ms", "end_ms", "p", "language"]);
	}

	/// A heard attempt keeps its words with it; a failure, silence or a replay
	/// keeps none; and a reader gets the words of the newest transcript that
	/// heard something, matching the text it takes.
	#[tokio::test]
	async fn a_heard_transcript_keeps_its_words_and_a_reader_takes_the_newest() {
		let pool = pool().await;
		note(&pool, A, "n1", "2026-10-25T01:00:00.000Z", b"audio").await;
		note(&pool, A, "quiet", "2026-10-25T02:00:00.000Z", b"hush").await;
		note(&pool, B, "n1", "2026-10-25T03:00:00.000Z", b"other").await;
		assert_eq!(words_of(&pool, A, "n1").await, [], "nothing heard yet");

		keep(&pool, A, "n1", &attempt(SMALL, Outcome::Failed)).await;
		assert_eq!(keep(&pool, A, "n1", &attempt(SMALL, heard("the particle 은 again"))).await, Kept::Added(2));
		assert_eq!(keep(&pool, A, "n1", &attempt(SMALL, heard("a replay"))).await, Kept::Ended);
		keep(&pool, A, "quiet", &attempt(SMALL, Outcome::Silent)).await;
		keep(&pool, B, "n1", &attempt(SMALL, heard("someone else"))).await;
		assert_eq!(words_of(&pool, A, "n1").await, said("the particle 은 again"));
		assert_eq!(words_of(&pool, A, "quiet").await, [], "silence has no words");
		assert_eq!(words_of(&pool, B, "n1").await, said("someone else"), "B's words are B's");

		// A better model, later, hears it again: its words are the ones read.
		let mut tx = pool.begin().await.unwrap();
		keep_attempt(&mut tx, A, "n1", &attempt(MEDIUM, heard("the particle 은, again")), "2026-10-26T09:00:00.000Z")
			.await
			.unwrap();
		tx.commit().await.unwrap();
		assert_eq!(words_of(&pool, A, "n1").await, said("the particle 은, again"));

		let kept: Vec<(String, i64, String)> = sqlx::query_as("SELECT model, word_index, text FROM voice_transcript_word WHERE subject_id = ?1 ORDER BY model DESC, word_index")
			.bind(A)
			.fetch_all(&pool)
			.await
			.unwrap();
		let kept: Vec<(&str, i64, &str)> = kept.iter().map(|(model, n, text)| (model.as_str(), *n, text.as_str())).collect();
		assert_eq!(
			kept,
			[
				(SMALL, 0, "the"),
				(SMALL, 1, "particle"),
				(SMALL, 2, "은"),
				(SMALL, 3, "again"),
				(MEDIUM, 0, "the"),
				(MEDIUM, 1, "particle"),
				(MEDIUM, 2, "은,"),
				(MEDIUM, 3, "again"),
			],
			"the older model's words are kept beside the newer's"
		);
	}

	/// A transcript heard before words were kept reads as having none, never
	/// as an older model's words, which would not match its text.
	#[tokio::test]
	async fn a_transcript_heard_without_words_reads_as_none() {
		let pool = pool().await;
		note(&pool, A, "n1", "2026-10-25T01:00:00.000Z", b"audio").await;
		keep(&pool, A, "n1", &attempt(SMALL, heard("hello there"))).await;
		let mut tx = pool.begin().await.unwrap();
		let wordless = Outcome::Heard {
			language: String::from("en"),
			language_p: 0.99,
			text: String::from("hello, there"),
			words: Vec::new(),
		};
		keep_attempt(&mut tx, A, "n1", &attempt(MEDIUM, wordless), "2026-10-26T09:00:00.000Z").await.unwrap();
		tx.commit().await.unwrap();
		assert_eq!(words_of(&pool, A, "n1").await, []);
	}

	/// A time the schema would refuse is held to it rather than failing the
	/// attempt, which would leave the note failing the same way every pass.
	#[tokio::test]
	async fn a_word_time_out_of_order_is_held_not_refused() {
		let pool = pool().await;
		note(&pool, A, "n1", "2026-10-25T01:00:00.000Z", b"audio").await;
		let odd = Outcome::Heard {
			language: String::from("en"),
			language_p: 0.99,
			text: String::from("early late"),
			words: vec![
				Word {
					text: String::from("early"),
					start_ms: -20,
					end_ms: 300,
					p: Some(1.5),
					language: Some(String::from("Korean")),
				},
				Word {
					text: String::from("late"),
					start_ms: 900,
					end_ms: 800,
					p: Some(f64::NAN),
					language: Some(String::from("ko")),
				},
			],
		};
		assert_eq!(keep(&pool, A, "n1", &attempt(SMALL, odd)).await, Kept::Added(1));
		let held: Vec<(i64, i64, Option<f64>, Option<String>)> = words_of(&pool, A, "n1")
			.await
			.into_iter()
			.map(|word| (word.start_ms, word.end_ms, word.p, word.language))
			.collect();
		assert_eq!(
			held,
			[(0, 300, Some(1.0), None), (900, 900, None, Some(String::from("ko")))],
			"a probability out of range is held to it, one that is not a number is none, and a language that is not a code is none"
		);
	}

	/// A transcript filed under a model carries that model's words with how
	/// sure it was of each (paulgsc/server#577), none where it gave no number.
	#[tokio::test]
	async fn a_models_words_read_back_with_how_sure_it_was() {
		let pool = pool().await;
		note(&pool, A, "n1", "2026-10-25T01:00:00.000Z", b"audio").await;
		keep(&pool, A, "n1", &attempt(SMALL, heard("hello there"))).await;
		let unsure = Outcome::Heard {
			language: String::from("en"),
			language_p: 0.99,
			text: String::from("hello there"),
			words: vec![
				Word {
					text: String::from("hello"),
					start_ms: 0,
					end_ms: 400,
					p: Some(0.3),
					language: Some(String::from("en")),
				},
				Word {
					text: String::from("there"),
					start_ms: 500,
					end_ms: 900,
					p: None,
					language: None,
				},
			],
		};
		keep(&pool, A, "n1", &attempt(MEDIUM, unsure.clone())).await;
		let mut conn = pool.acquire().await.unwrap();
		let Outcome::Heard { words, .. } = unsure else { unreachable!() };
		assert_eq!(words_heard_by(&mut conn, A, "n1", MEDIUM).await.unwrap(), words);
	}

	/// The schema holds the rules on its own, whatever writes to it.
	#[tokio::test]
	async fn the_schema_refuses_a_rewritten_word_or_a_time_out_of_order() {
		let pool = pool().await;
		note(&pool, A, "n1", "2026-10-25T01:00:00.000Z", b"audio").await;
		keep(&pool, A, "n1", &attempt(SMALL, heard("hello there"))).await;
		let refused = sqlx::query("UPDATE voice_transcript_word SET text = 'other'").execute(&pool).await;
		assert!(refused.unwrap_err().to_string().contains("append-only"), "no word is ever updated");
		for insert in [
			// A second word under a held index.
			"INSERT INTO voice_transcript_word VALUES ('subject-a', 'n1', 'ggml-small-q5_1.bin', 1, 'again', 2000, 2400, 0.9, 'en')",
			// Ending before it began.
			"INSERT INTO voice_transcript_word VALUES ('subject-a', 'n1', 'm', 0, 'x', 500, 400, 0.9, 'en')",
			// Before the note began.
			"INSERT INTO voice_transcript_word VALUES ('subject-a', 'n1', 'm', 0, 'x', -1, 400, 0.9, 'en')",
			// A place before the first.
			"INSERT INTO voice_transcript_word VALUES ('subject-a', 'n1', 'm', -1, 'x', 0, 400, 0.9, 'en')",
			// Surer than sure, or less than not at all.
			"INSERT INTO voice_transcript_word VALUES ('subject-a', 'n1', 'm', 0, 'x', 0, 400, 1.5, 'en')",
			"INSERT INTO voice_transcript_word VALUES ('subject-a', 'n1', 'm', 0, 'x', 0, 400, -0.1, 'en')",
			// A language that is not Whisper's code.
			"INSERT INTO voice_transcript_word VALUES ('subject-a', 'n1', 'm', 0, 'x', 0, 400, 0.9, 'Korean')",
			"INSERT INTO voice_transcript_word VALUES ('subject-a', 'n1', 'm', 0, 'x', 0, 400, 0.9, 'k')",
		] {
			assert!(sqlx::query(insert).execute(&pool).await.is_err(), "{insert}");
		}
	}

	#[tokio::test]
	async fn the_words_leave_with_the_account() {
		let pool = pool().await;
		for subject in [A, B] {
			sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?2)")
				.bind(subject)
				.bind(subject.as_bytes())
				.execute(&pool)
				.await
				.unwrap();
			note(&pool, subject, "n1", "2026-10-25T01:00:00.000Z", b"audio").await;
			keep(&pool, subject, "n1", &attempt(SMALL, heard("hello there"))).await;
		}
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		let left: Vec<String> = sqlx::query_scalar("SELECT DISTINCT subject_id FROM voice_transcript_word").fetch_all(&pool).await.unwrap();
		assert_eq!(left, [B], "A's words went with A's account, B's stayed");
	}
}
