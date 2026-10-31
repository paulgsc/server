//! Home's probes (paulgsc/some-ui#2383, CK4): `probe_run`, `probe_result`,
//! and the probe notes the transcription probe puts through the voice-note
//! path.
//!
//! `20261101000700_add_probe_to_voice_note.up.sql` and
//! `20261101000800_create_probe_run.up.sql` record the design; in short:
//!
//! - **A probe note is a voice note no person said.** It holds a test fixture
//!   (`file_host`'s `transcribe::probe` clip), said for `probe`, under
//!   [`SUBJECT`], which no account holds or can hold, so a reader that takes
//!   a subject never meets one and account deletion never takes one. The
//!   transcriber hears it like every note. The phone cannot send one
//!   ([`crate::describe`] refuses `of: probe`), and [`put_probe_note`] is its
//!   one writer.
//! - **A run, then at most one result, both append-only.** [`start`] writes
//!   the run and its note together, or joins the run already open: one run
//!   of a probe at a time. [`end`] writes the result once; the table's key
//!   refuses a second, and its triggers abort an update or a delete.
//! - **Nothing heard leaves through here.** A result holds a verdict, its
//!   timings and the engine's name. [`standing`] reads the probe note's
//!   transcript text for the caller to compare and keep only the score.

use crate::transcript::Outcome;
use crate::VoiceError;
use sha2::{Digest as _, Sha256};
use sqlx::SqliteConnection;

/// The subject every probe note is kept under: no account's, since an
/// account's subject is `subject-` and 32 hex digits
/// (`file_host`'s `subject::new_account_subject`).
pub const SUBJECT: &str = "probe";

/// What a probe note is said for, in `voice_note.of_kind`.
pub const OF_KIND: &str = "probe";

/// The probes there are, as `probe_run.probe` names them.
pub const PROBES: [&str; 1] = [TRANSCRIPTION];

/// The transcription probe: a known clip put through the voice-note path.
pub const TRANSCRIPTION: &str = "transcription";

/// Who asked for a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskedBy {
	/// Home's daily run.
	Schedule,
	/// The operator, from the web page.
	Page,
}

impl AskedBy {
	/// The stored and wire name.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Schedule => "schedule",
			Self::Page => "page",
		}
	}
}

/// One run of a probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
	/// `probe-` and 32 hex digits; the transcription probe's note id too.
	pub run_id: String,
	pub probe: String,
	/// `schedule` or `page`.
	pub asked_by: String,
	/// ISO-8601 UTC.
	pub started_at: String,
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq)]
pub struct Ended {
	/// `pass` or `fail`.
	pub verdict: &'static str,
	/// The transcript's outcome kind ([`Outcome::KINDS`]), or `timed_out`.
	pub outcome: &'static str,
	/// For `heard`: the word error rate against the expected words.
	pub word_error: Option<f64>,
	/// From the run's start to its ending.
	pub waited_ms: i64,
	/// How long the engine took on the attempt that ended it.
	pub took_ms: Option<i64>,
	pub model: Option<String>,
	pub engine: Option<String>,
}

/// A run and, once it has ended, its result as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct Kept {
	pub run: Run,
	pub result: Option<Ending>,
}

/// A result as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct Ending {
	pub verdict: String,
	pub outcome: String,
	pub word_error: Option<f64>,
	pub waited_ms: i64,
	pub took_ms: Option<i64>,
	pub model: Option<String>,
	pub engine: Option<String>,
	pub ended_at: String,
}

/// What [`start`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Started {
	/// A new run, with its note.
	New(Run),
	/// A run of this probe was already open: this is it, and nothing was
	/// written.
	Joined(Run),
}

impl Started {
	/// The run started or joined.
	#[must_use]
	pub const fn run(&self) -> &Run {
		match self {
			Self::New(run) | Self::Joined(run) => run,
		}
	}
}

/// Starts a transcription run as `run_id`, or joins the one already open.
///
/// The run is asked by `asked_by` as of `now`, and its note, holding `clip`,
/// is written with it under the run's id.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the read that decides and
/// the writes see one snapshot, and land together or not at all.
///
/// # Errors
/// [`VoiceError::Storage`] for any `sqlx` failure; nothing is written then.
pub async fn start(conn: &mut SqliteConnection, run_id: &str, asked_by: AskedBy, clip: Clip<'_>, now: &str) -> Result<Started, VoiceError> {
	if let Some(open) = open(conn, TRANSCRIPTION).await? {
		return Ok(Started::Joined(open));
	}
	let asked = asked_by.as_str();
	sqlx::query!(
		"INSERT INTO probe_run (run_id, probe, asked_by, started_at) VALUES (?1, ?2, ?3, ?4)",
		run_id,
		TRANSCRIPTION,
		asked,
		now
	)
	.execute(&mut *conn)
	.await?;
	put_probe_note(conn, run_id, clip, now).await?;
	Ok(Started::New(Run {
		run_id: run_id.to_owned(),
		probe: TRANSCRIPTION.to_owned(),
		asked_by: asked.to_owned(),
		started_at: now.to_owned(),
	}))
}

/// The clip a probe note holds.
#[derive(Debug, Clone, Copy)]
pub struct Clip<'a> {
	/// The note's `ref`: which clip, as its file names it.
	pub name: &'a str,
	/// One of [`crate::CONTENT_TYPES`].
	pub content_type: &'a str,
	pub body: &'a [u8],
}

/// Writes the probe note `note_id`, holding `clip`, under [`SUBJECT`] as of
/// `now`: its description and its audio together, as [`crate::put_note`]
/// writes a note the phone sends.
async fn put_probe_note(conn: &mut SqliteConnection, note_id: &str, clip: Clip<'_>, now: &str) -> Result<(), VoiceError> {
	let size = i64::try_from(clip.body.len()).unwrap_or(i64::MAX);
	let sha256 = hex::encode(Sha256::digest(clip.body));
	sqlx::query!(
		"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
		 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?5)",
		SUBJECT,
		note_id,
		OF_KIND,
		clip.name,
		now,
		clip.content_type,
		size,
		sha256
	)
	.execute(&mut *conn)
	.await?;
	sqlx::query!("INSERT INTO voice_note_audio (subject_id, note_id, body) VALUES (?1, ?2, ?3)", SUBJECT, note_id, clip.body)
		.execute(&mut *conn)
		.await?;
	Ok(())
}

/// The open run of `probe`, if any: the newest one with no result.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn open(conn: &mut SqliteConnection, probe: &str) -> Result<Option<Run>, VoiceError> {
	Ok(
		sqlx::query_as!(
			Run,
			r#"SELECT run.run_id AS "run_id!", run.probe, run.asked_by, run.started_at FROM probe_run AS run
			   WHERE run.probe = ?1 AND NOT EXISTS (SELECT 1 FROM probe_result AS result WHERE result.run_id = run.run_id)
			   ORDER BY run.started_at DESC LIMIT 1"#,
			probe
		)
		.fetch_optional(&mut *conn)
		.await?,
	)
}

/// When `probe` last started a run, if ever.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn last_started(conn: &mut SqliteConnection, probe: &str) -> Result<Option<String>, VoiceError> {
	Ok(
		sqlx::query_scalar!(r#"SELECT MAX(started_at) AS "at?: String" FROM probe_run WHERE probe = ?1"#, probe)
			.fetch_one(&mut *conn)
			.await?,
	)
}

/// Where the probe note of a run stands with `model`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
	/// Not heard yet, and not failed `attempts` times.
	Waiting,
	/// The model heard it, or found it silent, or failed on it for the last
	/// time: the attempt that ended it.
	Ended(Attempt),
}

/// The attempt that ended a probe note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
	/// [`Outcome::KINDS`]' name.
	pub outcome: &'static str,
	/// What was heard, for `heard`: compared by the caller, never kept in a
	/// result and never logged.
	pub text: Option<String>,
	pub took_ms: i64,
	pub engine: String,
	/// ISO-8601 UTC.
	pub heard_at: String,
}

/// Where the probe note `note_id` stands with `model`, which stops trying a
/// note after `attempts` failures (`file_host`'s `transcribe::pass::ATTEMPTS`).
///
/// # Errors
/// [`VoiceError::Unreadable`] for an outcome this crate did not write, and
/// any `sqlx` failure.
pub async fn standing(conn: &mut SqliteConnection, note_id: &str, model: &str, attempts: i64) -> Result<Standing, VoiceError> {
	let rows = sqlx::query!(
		r#"SELECT outcome, text, took_ms, engine, heard_at FROM voice_transcript
		   WHERE subject_id = ?1 AND note_id = ?2 AND model = ?3
		   ORDER BY attempt"#,
		SUBJECT,
		note_id,
		model
	)
	.fetch_all(&mut *conn)
	.await?;
	let tried = i64::try_from(rows.len()).unwrap_or(i64::MAX);
	let Some(last) = rows.into_iter().last() else {
		return Ok(Standing::Waiting);
	};
	let outcome = Outcome::KINDS.into_iter().find(|kind| *kind == last.outcome).ok_or(VoiceError::Unreadable)?;
	if !matches!(outcome, "heard" | "silent") && tried < attempts {
		return Ok(Standing::Waiting);
	}
	Ok(Standing::Ended(Attempt {
		outcome,
		text: last.text,
		took_ms: last.took_ms,
		engine: last.engine,
		heard_at: last.heard_at,
	}))
}

/// The runs with no result yet, oldest first, at most `limit`.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn unended(conn: &mut SqliteConnection, limit: i64) -> Result<Vec<Run>, VoiceError> {
	Ok(
		sqlx::query_as!(
			Run,
			r#"SELECT run.run_id AS "run_id!", run.probe, run.asked_by, run.started_at FROM probe_run AS run
			   WHERE NOT EXISTS (SELECT 1 FROM probe_result AS result WHERE result.run_id = run.run_id)
			   ORDER BY run.started_at, run.run_id LIMIT ?1"#,
			limit
		)
		.fetch_all(&mut *conn)
		.await?,
	)
}

/// Writes how run `run_id` ended, as of `now`, unless it has ended already.
/// Returns whether it wrote.
///
/// # Errors
/// Any `sqlx` failure, a result the schema refuses (a pass with nothing
/// heard) included.
pub async fn end(conn: &mut SqliteConnection, run_id: &str, ended: &Ended, now: &str) -> Result<bool, VoiceError> {
	let written = sqlx::query!(
		"INSERT INTO probe_result (run_id, verdict, outcome, word_error, waited_ms, took_ms, model, engine, ended_at)
		 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9
		 WHERE EXISTS (SELECT 1 FROM probe_run WHERE run_id = ?1)
		   AND NOT EXISTS (SELECT 1 FROM probe_result WHERE run_id = ?1)",
		run_id,
		ended.verdict,
		ended.outcome,
		ended.word_error,
		ended.waited_ms,
		ended.took_ms,
		ended.model,
		ended.engine,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok(written.rows_affected() == 1)
}

/// The newest `limit` runs of `probe`, newest first, each with its result
/// once it has one.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn recent(conn: &mut SqliteConnection, probe: &str, limit: i64) -> Result<Vec<Kept>, VoiceError> {
	let rows = sqlx::query!(
		r#"SELECT run.run_id AS "run_id!", run.probe, run.asked_by, run.started_at,
		          result.verdict AS "verdict?", result.outcome AS "outcome?", result.word_error,
		          result.waited_ms AS "waited_ms?", result.took_ms, result.model, result.engine,
		          result.ended_at AS "ended_at?"
		   FROM probe_run AS run LEFT JOIN probe_result AS result ON result.run_id = run.run_id
		   WHERE run.probe = ?1
		   ORDER BY run.started_at DESC, run.run_id DESC LIMIT ?2"#,
		probe,
		limit
	)
	.fetch_all(&mut *conn)
	.await?;
	Ok(
		rows
			.into_iter()
			.map(|row| {
				let result = match (row.verdict, row.outcome, row.waited_ms, row.ended_at) {
					(Some(verdict), Some(outcome), Some(waited_ms), Some(ended_at)) => Some(Ending {
						verdict,
						outcome,
						word_error: row.word_error,
						waited_ms,
						took_ms: row.took_ms,
						model: row.model,
						engine: row.engine,
						ended_at,
					}),
					_ => None,
				};
				Kept {
					run: Run {
						run_id: row.run_id,
						probe: row.probe,
						asked_by: row.asked_by,
						started_at: row.started_at,
					},
					result,
				}
			})
			.collect(),
	)
}

#[cfg(test)]
mod tests {
	use super::{end, last_started, open, recent, standing, start, unended, AskedBy, Clip, Ended, Standing, Started, SUBJECT, TRANSCRIPTION};
	use crate::transcript::{keep_attempt, waiting, Attempt, Outcome};
	use crate::{describe, Said, SaidBite};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	/// The migration before `probe` was a note's kind.
	const BEFORE_PROBE: i64 = 20_261_031_000_400;
	const MODEL: &str = "ggml-small-q5_1.bin";
	const CLIP: Clip<'static> = Clip {
		name: "quick-brown-fox-1",
		content_type: "audio/ogg",
		body: b"clip",
	};

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	async fn begin(pool: &SqlitePool, id: &str, asked_by: AskedBy, now: &str) -> Started {
		let mut tx = pool.begin().await.unwrap();
		let started = start(&mut tx, id, asked_by, CLIP, now).await.unwrap();
		tx.commit().await.unwrap();
		started
	}

	async fn attempt(pool: &SqlitePool, id: &str, outcome: Outcome, now: &str) {
		let mut conn = pool.acquire().await.unwrap();
		let attempt = Attempt {
			model: MODEL,
			engine: "whisper.cpp test",
			outcome,
			duration_ms: Some(2_970),
			took_ms: 1_200,
		};
		keep_attempt(&mut conn, SUBJECT, id, &attempt, now).await.unwrap();
	}

	fn heard(text: &str) -> Outcome {
		Outcome::Heard {
			language: String::from("en"),
			language_p: 0.99,
			text: String::from(text),
			words: Vec::new(),
		}
	}

	/// A run writes its note under the probe subject, said for `probe`, which
	/// the transcriber then finds like any note; a second start while it is
	/// open joins it and writes nothing.
	#[tokio::test]
	async fn a_run_puts_its_note_through_and_one_runs_at_a_time() {
		let pool = pool().await;
		let first = begin(&pool, "probe-1", AskedBy::Schedule, "2026-11-01T03:00:00.000Z").await;
		assert!(matches!(first, Started::New(_)));
		let again = begin(&pool, "probe-2", AskedBy::Page, "2026-11-01T03:00:05.000Z").await;
		assert_eq!(again, Started::Joined(first.run().clone()));

		let held: Vec<(String, String, String, String, i64)> = sqlx::query_as("SELECT subject_id, note_id, of_kind, of_ref, size FROM voice_note")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(held, [(SUBJECT.into(), "probe-1".into(), "probe".into(), "quick-brown-fox-1".into(), 4)]);
		let mut conn = pool.acquire().await.unwrap();
		let found = waiting(&mut conn, MODEL, 3, 20).await.unwrap();
		assert_eq!(found.len(), 1, "the transcriber hears a probe note like any note");
		assert_eq!(last_started(&mut conn, TRANSCRIPTION).await.unwrap().as_deref(), Some("2026-11-01T03:00:00.000Z"));
	}

	/// The phone cannot send a probe note: `describe` refuses `of: probe`.
	#[test]
	fn the_phone_cannot_say_a_note_is_a_probe() {
		let said = Said {
			of: Some("probe"),
			of_ref: Some("quick-brown-fox-1"),
			at: Some("2026-11-01T03:00:00Z"),
			content_type: Some("audio/ogg"),
			bite: SaidBite::default(),
		};
		let problems = describe("n1", said, b"clip").unwrap_err();
		assert!(problems.iter().any(|(field, _)| *field == "of"), "{problems:?}");
	}

	/// A note stands waiting until the model ends it: heard, silent, or
	/// failed for the last time.
	#[tokio::test]
	async fn a_probe_note_stands_waiting_until_its_attempts_end_it() {
		let pool = pool().await;
		begin(&pool, "probe-1", AskedBy::Page, "2026-11-01T03:00:00.000Z").await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(standing(&mut conn, "probe-1", MODEL, 2).await.unwrap(), Standing::Waiting);
		drop(conn);
		attempt(&pool, "probe-1", Outcome::Failed, "2026-11-01T03:01:00.000Z").await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(standing(&mut conn, "probe-1", MODEL, 2).await.unwrap(), Standing::Waiting, "one failure of two");
		drop(conn);
		attempt(&pool, "probe-1", Outcome::Failed, "2026-11-01T03:02:00.000Z").await;
		let mut conn = pool.acquire().await.unwrap();
		let Standing::Ended(last) = standing(&mut conn, "probe-1", MODEL, 2).await.unwrap() else {
			panic!("the last failure ends it");
		};
		assert_eq!((last.outcome, last.heard_at.as_str()), ("failed", "2026-11-01T03:02:00.000Z"));
		let failed = Ended {
			verdict: "fail",
			outcome: "failed",
			word_error: None,
			..passed()
		};
		assert!(end(&mut conn, "probe-1", &failed, "2026-11-01T03:02:01.000Z").await.unwrap());

		drop(conn);
		begin(&pool, "probe-h", AskedBy::Page, "2026-11-01T04:00:00.000Z").await;
		attempt(&pool, "probe-h", heard("The quick brown fox."), "2026-11-01T04:01:00.000Z").await;
		let mut conn = pool.acquire().await.unwrap();
		let Standing::Ended(last) = standing(&mut conn, "probe-h", MODEL, 3).await.unwrap() else {
			panic!("heard ends it");
		};
		assert_eq!((last.outcome, last.text.as_deref(), last.took_ms), ("heard", Some("The quick brown fox."), 1_200));
	}

	fn passed() -> Ended {
		Ended {
			verdict: "pass",
			outcome: "heard",
			word_error: Some(0.0),
			waited_ms: 61_000,
			took_ms: Some(1_200),
			model: Some(MODEL.into()),
			engine: Some("whisper.cpp test".into()),
		}
	}

	/// A run ends once: the result is kept, a second ending writes nothing,
	/// and the next start is a new run.
	#[tokio::test]
	async fn a_run_ends_once_and_reads_back_newest_first() {
		let pool = pool().await;
		begin(&pool, "probe-1", AskedBy::Schedule, "2026-11-01T03:00:00.000Z").await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(unended(&mut conn, 10).await.unwrap().len(), 1);
		assert!(end(&mut conn, "probe-1", &passed(), "2026-11-01T03:01:01.000Z").await.unwrap());
		assert!(
			!end(&mut conn, "probe-1", &passed(), "2026-11-01T03:02:00.000Z").await.unwrap(),
			"a second ending writes nothing"
		);
		assert!(!end(&mut conn, "probe-x", &passed(), "2026-11-01T03:02:00.000Z").await.unwrap(), "no run, no result");
		assert!(unended(&mut conn, 10).await.unwrap().is_empty());
		assert_eq!(open(&mut conn, TRANSCRIPTION).await.unwrap(), None);
		drop(conn);

		let next = begin(&pool, "probe-2", AskedBy::Page, "2026-11-02T03:00:00.000Z").await;
		assert!(matches!(next, Started::New(_)), "an ended run is not joined");
		let mut conn = pool.acquire().await.unwrap();
		let kept = recent(&mut conn, TRANSCRIPTION, 10).await.unwrap();
		assert_eq!(kept.iter().map(|k| k.run.run_id.as_str()).collect::<Vec<_>>(), ["probe-2", "probe-1"]);
		assert_eq!(kept[0].result, None);
		let result = kept[1].result.clone().unwrap();
		assert_eq!(
			(result.verdict.as_str(), result.waited_ms, result.ended_at.as_str()),
			("pass", 61_000, "2026-11-01T03:01:01.000Z")
		);
	}

	/// The schema keeps the record whole: no rewrite, no delete, and no pass
	/// without something heard.
	#[tokio::test]
	async fn the_schema_refuses_a_rewrite_a_delete_or_a_pass_with_nothing_heard() {
		let pool = pool().await;
		begin(&pool, "probe-1", AskedBy::Schedule, "2026-11-01T03:00:00.000Z").await;
		let mut conn = pool.acquire().await.unwrap();
		let silent_pass = Ended {
			outcome: "silent",
			word_error: None,
			..passed()
		};
		assert!(end(&mut conn, "probe-1", &silent_pass, "now").await.is_err());
		assert!(end(&mut conn, "probe-1", &passed(), "now").await.unwrap());
		for statement in [
			"UPDATE probe_run SET asked_by = 'page'",
			"DELETE FROM probe_run",
			"UPDATE probe_result SET verdict = 'fail'",
			"DELETE FROM probe_result",
		] {
			assert!(sqlx::query(statement).execute(&mut *conn).await.is_err(), "{statement}");
		}
	}

	/// The rebuild that widened `voice_note` for `probe` keeps every note
	/// held before it, and going back refuses once a probe note is held.
	#[tokio::test]
	async fn the_notes_rebuilt_for_probes_keep_what_was_held_before() {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		MIGRATOR.undo(&pool, BEFORE_PROBE).await.unwrap();
		sqlx::query(
			"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES ('subject-a', 'early', 'studio', 'p:s:1', 'then', 'audio/webm', 1, 'abc', 'now')",
		)
		.execute(&pool)
		.await
		.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		let held: Vec<(String, String)> = sqlx::query_as("SELECT note_id, of_kind FROM voice_note").fetch_all(&pool).await.unwrap();
		assert_eq!(held, [("early".into(), "studio".into())]);
		begin(&pool, "probe-1", AskedBy::Schedule, "2026-11-01T03:00:00.000Z").await;
		assert!(sqlx::query("UPDATE voice_note SET of_ref = 'x'").execute(&pool).await.is_err(), "still append-only");
		assert!(MIGRATOR.undo(&pool, BEFORE_PROBE).await.is_err(), "going back refuses a held probe note");
	}
}
