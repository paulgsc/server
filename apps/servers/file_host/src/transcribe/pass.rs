//! One pass of the transcriber: the notes the model has not heard, oldest
//! first, one at a time, each kept as an attempt in `voice_transcript`.
//!
//! - **Bounded.** A pass takes at most [`BATCH`] notes, each heard on a
//!   blocking thread by its deadline, and gives each one attempt, so a note
//!   that fails is retried a pass later, not at once, and at most
//!   [`ATTEMPTS`] times by one model (`voice_repo::transcript::waiting`).
//! - **Nothing outlives its account.** The attempt is written under
//!   `auth::DeletionLock`, and only for a note the subject still holds,
//!   checked in the transaction that writes (docs/identity.md invariant 9).
//!   The lock is never held while a note is heard.
//! - **A shutdown keeps nothing.** An attempt cancelled midway says nothing
//!   about the note, so it is not written, and the note waits for the next
//!   start.

use super::engine::{Cancelled, Hear, Heard};
use crate::auth::DeletionLock;
use sqlx::SqlitePool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use voice_repo::transcript::{self, Attempt, Kept, Outcome};
use voice_repo::VoiceError;

/// The most attempts one model makes on one note.
pub const ATTEMPTS: i64 = 3;

/// The most notes one pass hears.
pub const BATCH: i64 = 20;

/// What a pass did: the kind of each attempt it kept, in order.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Pass {
	pub kept: Vec<&'static str>,
	/// The server began shutting down during the pass.
	pub cancelled: bool,
}

/// Hears up to [`BATCH`] waiting notes with `hear`, each by `per_note`.
///
/// # Errors
/// A storage failure, which ends the pass; the attempts kept before it stay
/// kept, and the next pass carries on from them.
pub async fn run_once<H: Hear>(
	db: &SqlitePool,
	hear: &Arc<H>,
	deletion: &DeletionLock,
	per_note: Duration,
	cancel: &CancellationToken,
	now: &(dyn Fn() -> String + Sync),
) -> Result<Pass, VoiceError> {
	let mut pass = Pass::default();
	let waiting = {
		let mut conn = db.acquire().await?;
		transcript::waiting(&mut conn, hear.model(), ATTEMPTS, BATCH).await?
	};
	for note in waiting {
		if cancel.is_cancelled() {
			pass.cancelled = true;
			break;
		}
		let audio = {
			let mut conn = db.acquire().await?;
			transcript::audio(&mut conn, &note.subject, &note.note_id).await?
		};
		// Gone since the list was read: it went with its account.
		let Some(audio) = audio else { continue };
		let started = Instant::now();
		let heard = {
			let (hear, cancel) = (Arc::clone(hear), cancel.clone());
			let deadline = started + per_note;
			tokio::task::spawn_blocking(move || hear.hear(&audio, deadline, &cancel)).await
		};
		let heard = match heard {
			Ok(Ok(heard)) => heard,
			Ok(Err(Cancelled)) => {
				pass.cancelled = true;
				break;
			}
			// The engine panicked: a failure of this note, kept as one.
			Err(_) => Heard {
				outcome: Outcome::Failed,
				duration_ms: None,
			},
		};
		let took = started.elapsed();
		let kind = heard.outcome.kind();
		let attempt = Attempt {
			model: hear.model(),
			engine: hear.engine(),
			outcome: heard.outcome,
			duration_ms: heard.duration_ms,
			took_ms: i64::try_from(took.as_millis()).unwrap_or(i64::MAX),
		};
		if let Kept::Added(_) = keep(db, deletion, &note.subject, &note.note_id, &attempt, &now()).await? {
			crate::metrics::transcribe::record_attempt(kind, took);
			pass.kept.push(kind);
		}
	}
	Ok(pass)
}

/// One write, under the deletion hold (docs/identity.md invariant 9):
/// `keep_attempt` checks the note is still held in the transaction that
/// writes, and the hold keeps a deletion from landing between the two.
async fn keep(db: &SqlitePool, deletion: &DeletionLock, subject: &str, note_id: &str, attempt: &Attempt<'_>, now: &str) -> Result<Kept, VoiceError> {
	let _hold = deletion.hold().await;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let kept = transcript::keep_attempt(&mut tx, subject, note_id, attempt, now).await?;
	tx.commit().await?;
	Ok(kept)
}

#[cfg(test)]
mod tests {
	use super::{run_once, Pass, ATTEMPTS, BATCH};
	use crate::auth::DeletionLock;
	use crate::transcribe::engine::{Cancelled, Hear, Heard};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
	use std::sync::{Arc, Mutex};
	use std::time::{Duration, Instant};
	use tokio_util::sync::CancellationToken;
	use voice_repo::transcript::Outcome;
	use voice_repo::{describe, put_note, Said, SaidBite};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";
	const B: &str = "subject-b";
	/// What the fake hears in every note that is not `silence` or `noise`.
	const SAID: &str = "the particle 은 again, said out loud";

	/// Hears the audio's own bytes: `silence` is silent, `noise` does not
	/// decode, `slow` runs to its deadline, `stop` shuts the server down
	/// midway, and anything else is heard as [`SAID`].
	struct Fake {
		heard: Mutex<Vec<Vec<u8>>>,
		cancel: CancellationToken,
	}

	impl Fake {
		fn new(cancel: &CancellationToken) -> Arc<Self> {
			Arc::new(Self {
				heard: Mutex::new(Vec::new()),
				cancel: cancel.clone(),
			})
		}

		fn heard(&self) -> Vec<String> {
			self.heard.lock().unwrap().iter().map(|audio| String::from_utf8_lossy(audio).into_owned()).collect()
		}
	}

	impl Hear for Fake {
		fn model(&self) -> &'static str {
			"ggml-small-q5_1.bin"
		}

		fn engine(&self) -> &'static str {
			"whisper.cpp test"
		}

		fn hear(&self, audio: &[u8], deadline: Instant, cancel: &CancellationToken) -> Result<Heard, Cancelled> {
			self.heard.lock().unwrap().push(audio.to_vec());
			let outcome = match audio {
				b"silence" => Outcome::Silent,
				b"noise" => Outcome::Unreadable,
				b"slow" => {
					while Instant::now() < deadline {
						std::thread::sleep(Duration::from_millis(5));
					}
					Outcome::Deadline
				}
				b"stop" => {
					self.cancel.cancel();
					assert!(cancel.is_cancelled(), "the pass's own token reaches the engine");
					return Err(Cancelled);
				}
				_ => Outcome::Heard {
					language: String::from("en"),
					language_p: 0.97,
					text: String::from(SAID),
					words: Vec::new(),
				},
			};
			Ok(Heard {
				outcome,
				duration_ms: Some(28_400),
			})
		}
	}

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	async fn note(pool: &SqlitePool, subject: &str, id: &str, stored_at: &str, audio: &[u8]) {
		let said = Said {
			of: Some("reflection"),
			of_ref: Some("session-7f2a"),
			at: Some("2026-10-25T03:10:00Z"),
			content_type: Some("audio/webm"),
			bite: SaidBite::default(),
		};
		let mut tx = pool.begin().await.unwrap();
		put_note(&mut tx, subject, &describe(id, said, audio).unwrap(), audio, stored_at).await.unwrap();
		tx.commit().await.unwrap();
	}

	async fn pass(pool: &SqlitePool, fake: &Arc<Fake>, per_note: Duration, cancel: &CancellationToken) -> Pass {
		run_once(pool, fake, &DeletionLock::default(), per_note, cancel, &|| String::from("2026-10-25T16:00:00.000Z"))
			.await
			.unwrap()
	}

	async fn rows(pool: &SqlitePool) -> Vec<(String, String, i64, String, Option<String>)> {
		sqlx::query_as("SELECT subject_id, note_id, attempt, outcome, text FROM voice_transcript ORDER BY subject_id, note_id, attempt")
			.fetch_all(pool)
			.await
			.unwrap()
	}

	/// Every stored note gets a transcript or a failure, oldest first; a
	/// second pass adds nothing for what was heard, and a failure is tried
	/// again a pass later, at most `ATTEMPTS` times.
	#[tokio::test]
	async fn every_note_gets_a_row_and_a_failure_is_retried_a_few_times() {
		let pool = pool().await;
		note(&pool, A, "spoken", "2026-10-25T03:00:00.000Z", b"words").await;
		note(&pool, B, "quiet", "2026-10-25T02:00:00.000Z", b"silence").await;
		note(&pool, A, "broken", "2026-10-25T01:00:00.000Z", b"noise").await;
		let cancel = CancellationToken::new();
		let fake = Fake::new(&cancel);

		let first = pass(&pool, &fake, Duration::from_secs(60), &cancel).await;
		assert_eq!(first.kept, ["unreadable", "silent", "heard"], "oldest first");
		assert_eq!(fake.heard(), ["noise", "silence", "words"]);

		for _ in 1..ATTEMPTS + 2 {
			pass(&pool, &fake, Duration::from_secs(60), &cancel).await;
		}
		assert_eq!(fake.heard().iter().filter(|audio| *audio == "noise").count(), 3, "three tries and no more");
		assert_eq!(fake.heard().len(), 5, "heard and silent notes are never heard again");

		let written = rows(&pool).await;
		let expected = [
			(A, "broken", 1, "unreadable", None),
			(A, "broken", 2, "unreadable", None),
			(A, "broken", 3, "unreadable", None),
			(A, "spoken", 1, "heard", Some(SAID)),
			(B, "quiet", 1, "silent", None),
		];
		let expected: Vec<_> = expected
			.into_iter()
			.map(|(subject, id, n, kind, text)| (subject.to_owned(), id.to_owned(), n, kind.to_owned(), text.map(str::to_owned)))
			.collect();
		assert_eq!(written, expected);
	}

	/// The engine is handed the per-note deadline, and running past it is
	/// kept as `deadline`.
	#[tokio::test]
	async fn a_slow_note_is_kept_as_past_its_deadline() {
		let pool = pool().await;
		note(&pool, A, "n1", "2026-10-25T01:00:00.000Z", b"slow").await;
		let cancel = CancellationToken::new();
		let fake = Fake::new(&cancel);
		let started = Instant::now();
		assert_eq!(pass(&pool, &fake, Duration::from_millis(50), &cancel).await.kept, ["deadline"]);
		assert!(started.elapsed() < Duration::from_secs(5));
	}

	/// A shutdown midway keeps nothing about the note it interrupted and hears
	/// nothing after it.
	#[tokio::test]
	async fn a_shutdown_keeps_nothing_for_the_note_it_interrupts() {
		let pool = pool().await;
		note(&pool, A, "first", "2026-10-25T01:00:00.000Z", b"words").await;
		note(&pool, A, "second", "2026-10-25T02:00:00.000Z", b"stop").await;
		note(&pool, A, "third", "2026-10-25T03:00:00.000Z", b"words").await;
		let cancel = CancellationToken::new();
		let fake = Fake::new(&cancel);
		let done = pass(&pool, &fake, Duration::from_secs(60), &cancel).await;
		assert_eq!(
			done,
			Pass {
				kept: vec!["heard"],
				cancelled: true
			}
		);
		assert_eq!(fake.heard(), ["words", "stop"]);
		assert_eq!(rows(&pool).await.len(), 1);
	}

	/// A pass takes at most `BATCH` notes; the rest wait for the next.
	#[tokio::test]
	async fn a_pass_hears_at_most_a_batch() {
		let pool = pool().await;
		for n in 0..=BATCH {
			let id = String::from("n") + &n.to_string();
			let at = String::from("2026-10-25T01:00:") + &format_args!("{n:02}").to_string() + ".000Z";
			note(&pool, A, &id, &at, b"words").await;
		}
		let cancel = CancellationToken::new();
		let fake = Fake::new(&cancel);
		assert_eq!(i64::try_from(pass(&pool, &fake, Duration::from_secs(60), &cancel).await.kept.len()).unwrap(), BATCH);
		assert_eq!(pass(&pool, &fake, Duration::from_secs(60), &cancel).await.kept.len(), 1);
	}

	/// Deletes every account while it hears, as a deletion landing mid-note
	/// would.
	struct Deleting(SqlitePool);
	impl Hear for Deleting {
		fn model(&self) -> &'static str {
			"m"
		}
		fn engine(&self) -> &'static str {
			"e"
		}
		fn hear(&self, _: &[u8], _: Instant, _: &CancellationToken) -> Result<Heard, Cancelled> {
			let pool = self.0.clone();
			tokio::runtime::Handle::current().block_on(async move {
				sqlx::query("DELETE FROM account").execute(&pool).await.unwrap();
			});
			Ok(Heard {
				outcome: Outcome::Silent,
				duration_ms: Some(1),
			})
		}
	}

	/// A note whose account was deleted while it was heard keeps nothing.
	#[tokio::test]
	async fn nothing_is_kept_for_a_note_that_left_with_its_account() {
		let pool = pool().await;
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, x'00')")
			.bind(A)
			.execute(&pool)
			.await
			.unwrap();
		note(&pool, A, "n1", "2026-10-25T01:00:00.000Z", b"words").await;

		let done = run_once(
			&pool,
			&Arc::new(Deleting(pool.clone())),
			&DeletionLock::default(),
			Duration::from_secs(60),
			&CancellationToken::new(),
			&|| String::from("now"),
		)
		.await
		.unwrap();
		assert!(done.kept.is_empty());
		assert!(rows(&pool).await.is_empty());
	}

	/// docs/identity.md invariant 23: what was heard, and the audio, reach no
	/// log line; a pass and its summary log kinds and counts only.
	#[tokio::test]
	async fn no_transcript_or_audio_reaches_a_log_line() {
		let (captured, _guard) = crate::privacy::capture();
		let pool = pool().await;
		note(&pool, A, "n1", "2026-10-25T01:00:00.000Z", b"words").await;
		note(&pool, A, "n2", "2026-10-25T02:00:00.000Z", b"noise").await;
		let cancel = CancellationToken::new();
		let fake = Fake::new(&cancel);
		let done = pass(&pool, &fake, Duration::from_secs(60), &cancel).await;
		crate::transcribe::log_pass(&Ok(done));
		let lines = captured.lines();
		assert!(lines.iter().any(|line| line.contains("heard")), "sanity check: the summary was captured: {lines:?}");
		for line in &lines {
			for heard in [SAID, "words", "noise", "은"] {
				assert!(!line.contains(heard), "what a note said, or its audio, reached a log line");
			}
		}
	}
}
