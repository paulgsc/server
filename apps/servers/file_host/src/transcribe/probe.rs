//! Home checks its own transcription end to end with a known clip.
//!
//! The story is paulgsc/some-ui#2383 (CK4, part of the CHECKS epic,
//! paulgsc/some-ui#2369): a known clip goes in as a probe note, the
//! transcriber hears it like any note, and what it heard is compared with
//! the clip's words.
//!
//! **What a run is.** [`start`] writes a run and its probe note together
//! (`voice_repo::probe`), or joins the run already open, so one run is on its
//! way at a time. The note is heard by the real pass and the real engine
//! ([`super::pass`]), oldest first like every note, so the time a run waits
//! includes any notes ahead of it. [`settle`] then ends each open run whose
//! note the model has ended: `pass` when it heard the clip's words within
//! [`TOLERANCE`] word error, `fail` otherwise, or `fail` as `timed_out` once
//! [`BOUND`] passes with no ending. Each result keeps the verdict, the time
//! to transcript, the engine's time and its name, append-only.
//!
//! **When it runs.** Daily ([`EVERY`], the owner's decision C4 on the epic)
//! and on demand from the web page (`POST /api/v1/probes/transcription`,
//! operator only). Both ride the transcriber's own tick
//! (`transcribe::spawn`): before each pass, [`start_if_due`]; after it,
//! [`settle`]. No loop of its own (paulgsc/server#485 asks for fewer), and
//! nothing runs while transcription is off.
//!
//! **No person's data.** The clip is a test fixture: speech synthesized from
//! [`EXPECTED`] by ffmpeg's `flite` filter, never a person's recording,
//! regenerated with
//!
//! ```text
//! ffmpeg -f lavfi -i "flite=text='The quick brown fox jumps over the lazy dog.':voice=slt" \
//!   -ac 1 -ar 16000 -c:a libopus -b:a 24k -map_metadata -1 -fflags +bitexact \
//!   -flags:a +bitexact quick-brown-fox-1.ogg
//! ```
//!
//! Its note is kept under `voice_repo::probe::SUBJECT`, which no account
//! holds, so no routine, mailbox filing or count of a subject's notes meets
//! it (docs/identity.md invariant 29). A run logs its verdict, outcome and
//! timings, never what was heard.

use super::pass::ATTEMPTS;
use chrono::{DateTime, SecondsFormat, Utc};
use rand::RngCore;
use sqlx::SqlitePool;
use std::time::Duration;
use tracing::{error, info};
use voice_repo::probe::{self, AskedBy, Attempt, Clip, Ended, Standing, Started, TRANSCRIPTION};
use voice_repo::VoiceError;

/// What the probe routes (`handlers::probes`) need of the transcriber:
/// whether it runs here, which is whether anything would hear a probe note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Probing {
	pub transcription: bool,
}

/// The known clip: "The quick brown fox jumps over the lazy dog.", about
/// three seconds, Opus in Ogg.
pub const CLIP: Clip<'static> = Clip {
	name: "quick-brown-fox-1",
	content_type: "audio/ogg",
	body: include_bytes!("probe/quick-brown-fox-1.ogg"),
};

/// The words the clip says, as [`words`] reads them.
pub const EXPECTED: &str = "the quick brown fox jumps over the lazy dog";

/// The most word error a heard clip may have and pass: two of its nine words
/// wrong.
pub const TOLERANCE: f64 = 0.25;

/// How often a run is due without being asked for.
pub const EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// How long a run waits for its note to be heard before it ends as
/// `timed_out`. A pass hears at most `pass::BATCH` notes of a minute or so
/// each, so an hour is a backlog no healthy home has.
pub const BOUND: Duration = Duration::from_secs(60 * 60);

/// The most open runs one [`settle`] reads. One is open at a time
/// ([`start`]), so this only bounds a pass if that ever stops holding.
const SETTLE_LIMIT: i64 = 10;

fn stamp(at: DateTime<Utc>) -> String {
	at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn instant(at: &str) -> Option<DateTime<Utc>> {
	DateTime::parse_from_rfc3339(at).ok().map(|at| at.with_timezone(&Utc))
}

fn millis(from: DateTime<Utc>, to: DateTime<Utc>) -> i64 {
	(to - from).num_milliseconds().max(0)
}

/// A new run's id: `probe-` and 32 random hex digits, also its note's id.
fn run_id() -> String {
	let mut bytes = [0_u8; 16];
	rand::rng().fill_bytes(&mut bytes);
	String::from("probe-") + &hex::encode(bytes)
}

/// Starts a transcription run asked by `asked_by` as of `now`, or joins the
/// one already open.
///
/// # Errors
/// Any storage failure; nothing is written then.
pub async fn start(db: &SqlitePool, asked_by: AskedBy, now: DateTime<Utc>) -> Result<Started, VoiceError> {
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let started = probe::start(&mut tx, &run_id(), asked_by, CLIP, &stamp(now)).await?;
	tx.commit().await?;
	Ok(started)
}

/// Starts a scheduled run when none has started in the last [`EVERY`].
///
/// # Errors
/// Any storage failure.
pub async fn start_if_due(db: &SqlitePool, now: DateTime<Utc>) -> Result<Option<Started>, VoiceError> {
	let last = {
		let mut conn = db.acquire().await?;
		probe::last_started(&mut conn, TRANSCRIPTION).await?
	};
	let every = chrono::Duration::from_std(EVERY).unwrap_or(chrono::Duration::MAX);
	// A start that does not read back is no reason to stop checking.
	if last.as_deref().and_then(instant).is_some_and(|at| now - at < every) {
		return Ok(None);
	}
	start(db, AskedBy::Schedule, now).await.map(Some)
}

/// Ends every open run whose note `model` has ended, or that has waited past
/// [`BOUND`], and returns each verdict written.
///
/// # Errors
/// Any storage failure; the results written before it stay written.
pub async fn settle(db: &SqlitePool, model: &str, now: DateTime<Utc>) -> Result<Vec<Ended>, VoiceError> {
	let mut conn = db.acquire().await?;
	let mut written = Vec::new();
	for run in probe::unended(&mut conn, SETTLE_LIMIT).await? {
		let started = instant(&run.started_at).unwrap_or(now);
		let ended = match probe::standing(&mut conn, &run.run_id, model, ATTEMPTS).await? {
			Standing::Ended(attempt) => judge(&attempt, model, started),
			Standing::Waiting if millis(started, now) >= i64::try_from(BOUND.as_millis()).unwrap_or(i64::MAX) => timed_out(started, now),
			Standing::Waiting => continue,
		};
		if probe::end(&mut conn, &run.run_id, &ended, &stamp(now)).await? {
			crate::metrics::probe::record_ended(TRANSCRIPTION, ended.verdict, now);
			written.push(ended);
		}
	}
	Ok(written)
}

/// The verdict on the attempt that ended a run's note: `pass` only when it
/// heard the clip's words within [`TOLERANCE`].
#[must_use]
pub fn judge(attempt: &Attempt, model: &str, started: DateTime<Utc>) -> Ended {
	let word_error = attempt.text.as_deref().filter(|_| attempt.outcome == "heard").map(|text| word_error(EXPECTED, text));
	let verdict = if word_error.is_some_and(|error| error <= TOLERANCE) { "pass" } else { "fail" };
	let ended = instant(&attempt.heard_at).unwrap_or(started);
	Ended {
		verdict,
		outcome: attempt.outcome,
		word_error,
		waited_ms: millis(started, ended),
		took_ms: Some(attempt.took_ms),
		model: Some(model.to_owned()),
		engine: Some(attempt.engine.clone()),
	}
}

fn timed_out(started: DateTime<Utc>, now: DateTime<Utc>) -> Ended {
	Ended {
		verdict: "fail",
		outcome: "timed_out",
		word_error: None,
		waited_ms: millis(started, now),
		took_ms: None,
		model: None,
		engine: None,
	}
}

/// `text`'s words, lowercased, without punctuation: letters, digits and
/// apostrophes, split on anything else.
#[must_use]
pub fn words(text: &str) -> Vec<String> {
	text
		.split(|c: char| !(c.is_alphanumeric() || c == '\''))
		.filter(|word| !word.is_empty())
		.map(str::to_lowercase)
		.collect()
}

/// The word error rate of `heard` against `expected`: the fewest words
/// substituted, dropped or added to turn one into the other, over the
/// expected words' count. 0 is word for word; it can pass 1.
#[must_use]
pub fn word_error(expected: &str, heard: &str) -> f64 {
	let (expected, heard) = (words(expected), words(heard));
	if expected.is_empty() {
		return if heard.is_empty() { 0.0 } else { 1.0 };
	}
	let mut row: Vec<usize> = (0..=heard.len()).collect();
	for (i, want) in expected.iter().enumerate() {
		let mut diagonal = row[0];
		row[0] = i + 1;
		for (j, got) in heard.iter().enumerate() {
			let substitute = diagonal + usize::from(want != got);
			diagonal = row[j + 1];
			row[j + 1] = substitute.min(row[j] + 1).min(diagonal + 1);
		}
	}
	#[allow(clippy::cast_precision_loss, reason = "word counts of a few words")]
	let rate = row[heard.len()] as f64 / expected.len() as f64;
	rate
}

/// What the transcriber's tick does before its pass: a scheduled run, when
/// one is due. A failure is logged and the pass goes on.
pub async fn before_pass(db: &SqlitePool) {
	match start_if_due(db, Utc::now()).await {
		Ok(Some(Started::New(_))) => info!(probe = TRANSCRIPTION, asked_by = "schedule", "probe run started"),
		Ok(Some(Started::Joined(_)) | None) => {}
		Err(err) => error!(probe = TRANSCRIPTION, error = super::kind(&err), "probe run could not start"),
	}
}

/// What the transcriber's tick does after its pass: end the runs it can. Each
/// result is logged by its verdict, outcome and timings, never by what was
/// heard.
pub async fn after_pass(db: &SqlitePool, model: &str) {
	match settle(db, model, Utc::now()).await {
		Ok(ended) => {
			for run in ended {
				info!(
					probe = TRANSCRIPTION,
					verdict = run.verdict,
					outcome = run.outcome,
					word_error = run.word_error,
					waited_ms = run.waited_ms,
					took_ms = run.took_ms,
					"probe run ended"
				);
			}
		}
		Err(err) => error!(probe = TRANSCRIPTION, error = super::kind(&err), "probe runs could not be settled"),
	}
}

#[cfg(test)]
mod tests {
	use super::{judge, settle, start, start_if_due, word_error, words, BOUND, EVERY, EXPECTED, TOLERANCE};
	use crate::auth::DeletionLock;
	use crate::transcribe::engine::{Cancelled, Hear, Heard};
	use crate::transcribe::pass::run_once;
	use chrono::{DateTime, Utc};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
	use std::sync::Arc;
	use std::time::{Duration, Instant};
	use tokio_util::sync::CancellationToken;
	use voice_repo::probe::{recent, AskedBy, Attempt, Started, TRANSCRIPTION};
	use voice_repo::transcript::Outcome;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	/// Hears every note as `said`, or finds it silent when `said` is `None`.
	struct Fake {
		said: Option<&'static str>,
	}

	impl Hear for Fake {
		fn model(&self) -> &'static str {
			"ggml-small-q5_1.bin"
		}

		fn engine(&self) -> &'static str {
			"whisper.cpp test"
		}

		fn hear(&self, _: &[u8], _: Instant, _: &CancellationToken) -> Result<Heard, Cancelled> {
			let outcome = self.said.map_or(Outcome::Silent, |said| Outcome::Heard {
				language: String::from("en"),
				language_p: 0.99,
				text: String::from(said),
				words: Vec::new(),
			});
			Ok(Heard {
				outcome,
				duration_ms: Some(2_970),
			})
		}
	}

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn at(text: &str) -> DateTime<Utc> {
		DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc)
	}

	async fn hear(pool: &SqlitePool, fake: Fake) {
		run_once(pool, &Arc::new(fake), &DeletionLock::default(), Duration::from_secs(60), &CancellationToken::new(), &|| {
			String::from("2026-11-01T03:01:00.000Z")
		})
		.await
		.unwrap();
	}

	#[test]
	fn words_are_compared_without_case_or_punctuation() {
		assert_eq!(words("The quick, brown FOX!"), ["the", "quick", "brown", "fox"]);
		assert!(word_error(EXPECTED, "The quick brown fox jumps over the lazy dog.").abs() < f64::EPSILON);
		let one_wrong = word_error(EXPECTED, "the quick brown box jumps over the lazy dog");
		assert!((one_wrong - 1.0 / 9.0).abs() < 1e-9, "{one_wrong}");
		let dropped = word_error(EXPECTED, "the quick fox jumps over the dog");
		assert!((dropped - 2.0 / 9.0).abs() < 1e-9, "two words dropped: {dropped}");
		assert!(word_error(EXPECTED, "").abs() > TOLERANCE);
		assert!(word_error(EXPECTED, "thank you").abs() > TOLERANCE);
	}

	fn attempt(outcome: &'static str, text: Option<&str>) -> Attempt {
		Attempt {
			outcome,
			text: text.map(str::to_owned),
			took_ms: 900,
			engine: String::from("whisper.cpp 1.8.3"),
			heard_at: String::from("2026-11-01T03:01:00.000Z"),
		}
	}

	/// A pass needs the clip's words within the tolerance; silence, a
	/// failure or the wrong words fail, and the wait runs from the start to
	/// the hearing.
	#[test]
	fn a_run_passes_only_on_the_clips_words() {
		let started = at("2026-11-01T03:00:00.000Z");
		let passed = judge(&attempt("heard", Some("The quick brown fox jumps over the lazy dog.")), "m", started);
		assert_eq!((passed.verdict, passed.waited_ms, passed.took_ms), ("pass", 60_000, Some(900)));
		assert_eq!(passed.engine.as_deref(), Some("whisper.cpp 1.8.3"));
		assert_eq!(judge(&attempt("heard", Some("the quick fox")), "m", started).verdict, "fail");
		let silent = judge(&attempt("silent", None), "m", started);
		assert_eq!((silent.verdict, silent.word_error), ("fail", None));
		assert_eq!(judge(&attempt("failed", None), "m", started).verdict, "fail");
	}

	/// End to end through the real pass: a run's note is heard and the run
	/// ends with a verdict; the next is due a day later, not before.
	#[tokio::test]
	async fn a_scheduled_run_is_heard_by_the_pass_and_ends_with_a_verdict() {
		let pool = pool().await;
		let first = start_if_due(&pool, at("2026-11-01T03:00:00.000Z")).await.unwrap();
		assert!(matches!(first, Some(Started::New(_))), "the first run is due at once");
		assert!(
			settle(&pool, "ggml-small-q5_1.bin", at("2026-11-01T03:00:30.000Z")).await.unwrap().is_empty(),
			"not heard yet"
		);

		hear(
			&pool,
			Fake {
				said: Some(" The quick brown fox jumps over the lazy dog."),
			},
		)
		.await;
		let ended = settle(&pool, "ggml-small-q5_1.bin", at("2026-11-01T03:01:05.000Z")).await.unwrap();
		assert_eq!(ended.len(), 1);
		assert_eq!((ended[0].verdict, ended[0].outcome, ended[0].waited_ms), ("pass", "heard", 60_000));

		assert!(start_if_due(&pool, at("2026-11-02T02:59:59.000Z")).await.unwrap().is_none(), "not a day yet");
		let every = chrono::Duration::from_std(EVERY).unwrap();
		assert!(start_if_due(&pool, at("2026-11-01T03:00:00.000Z") + every).await.unwrap().is_some());
		let mut conn = pool.acquire().await.unwrap();
		let kept = recent(&mut conn, TRANSCRIPTION, 10).await.unwrap();
		assert_eq!(kept.len(), 2);
		assert_eq!(kept[1].result.as_ref().map(|r| r.verdict.as_str()), Some("pass"));
	}

	/// A run the page asks for while one is open joins it; a silent hearing
	/// fails it; a run nobody hears fails as timed out once the bound passes.
	#[tokio::test]
	async fn a_page_run_joins_the_open_one_and_a_run_never_heard_times_out() {
		let pool = pool().await;
		let open = start(&pool, AskedBy::Page, at("2026-11-01T03:00:00.000Z")).await.unwrap();
		let again = start(&pool, AskedBy::Page, at("2026-11-01T03:00:10.000Z")).await.unwrap();
		assert_eq!(again, Started::Joined(open.run().clone()));
		hear(&pool, Fake { said: None }).await;
		let ended = settle(&pool, "ggml-small-q5_1.bin", at("2026-11-01T03:01:05.000Z")).await.unwrap();
		assert_eq!((ended[0].verdict, ended[0].outcome), ("fail", "silent"));

		start(&pool, AskedBy::Page, at("2026-11-01T04:00:00.000Z")).await.unwrap();
		let bound = chrono::Duration::from_std(BOUND).unwrap();
		let before = at("2026-11-01T04:00:00.000Z") + bound - chrono::Duration::seconds(1);
		assert!(settle(&pool, "ggml-small-q5_1.bin", before).await.unwrap().is_empty());
		let ended = settle(&pool, "ggml-small-q5_1.bin", at("2026-11-01T04:00:00.000Z") + bound).await.unwrap();
		assert_eq!((ended[0].verdict, ended[0].outcome, ended[0].took_ms), ("fail", "timed_out", None));
	}

	/// docs/identity.md invariant 29: a run's log lines carry its verdict and
	/// timings, never what was heard.
	#[tokio::test]
	async fn what_a_probe_heard_reaches_no_log_line() {
		let (captured, _guard) = crate::privacy::capture();
		let pool = pool().await;
		start(&pool, AskedBy::Schedule, Utc::now()).await.unwrap();
		hear(
			&pool,
			Fake {
				said: Some("the quick brown fox, overheard"),
			},
		)
		.await;
		super::after_pass(&pool, "ggml-small-q5_1.bin").await;
		let lines = captured.lines();
		assert!(lines.iter().any(|line| line.contains("probe run ended")), "sanity check: the result was logged: {lines:?}");
		for line in &lines {
			assert!(!line.contains("overheard"), "what a probe heard reached a log line: {line}");
		}
	}
}
