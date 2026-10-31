//! The owner's own words for a Studio take (paulgsc/some-ui#2322, STUDIO
//! ST24): what home heard in a take, read back for the phone, and the
//! owner's fix of it, kept as one more transcript beside the engines'.
//!
//! - **The owner's words are one more hearing** (owner, paulgsc/some-ui#2248,
//!   "Decided" 12). Each save is a `voice_transcript` row of its own, written
//!   by [`crate::transcript::keep_attempt`] as an engine's is, under the
//!   model `owner:<n>` ([`model`]) and the engine [`ENGINE`], with its words
//!   in `voice_transcript_word`. So no table is added, a machine's
//!   transcript is never rewritten, and the Jobs mailbox poller files the
//!   owner's like a better model's, under a new name.
//! - **Append-only.** `n` names one save. The same words under a held `n`
//!   change nothing, and other words under it are refused
//!   ([`Refused::Rewritten`]); a later fix is a new `n`, and every earlier
//!   one stays.
//! - **The owner's words are the truth.** A reader of a take's words takes
//!   the owner's newest save where there is one, then the newest engine
//!   hearing ([`Hearings::chosen`], and `transcript::words`, in one order).
//! - **Only a Studio take.** Both halves answer for a note said for `studio`
//!   that the subject holds, and for nothing else.
//! - **Built on a hearing.** A save names the hearing it corrects
//!   (`basedOn`), which must have heard the take; the owner's transcript
//!   keeps that hearing's language, its probability and the audio's length,
//!   since the owner changes words, not those.

use crate::transcript::{keep_attempt, words_heard_by, Attempt, Kept, Outcome, Word};
use crate::{Change, Problem, VoiceError};
use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;

/// The engine column of every transcript the owner wrote: who made it.
pub const ENGINE: &str = "owner";

/// The most saves one take may have, and so the largest `n`.
pub const MAX_SAVES: i64 = 9_999;

/// The most words one save may hold: ten minutes at twenty a second, as the
/// mailbox takes a transcript.
pub const MAX_WORDS: usize = 12_000;

/// The most characters one word may hold.
pub const MAX_WORD_CHARS: usize = 100;

/// The model name the owner's `n`th save of a take goes under.
#[must_use]
pub fn model(n: i64) -> String {
	String::from(ENGINE) + ":" + &n.to_string()
}

/// Whose a hearing is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum By {
	/// Home's transcriber heard it.
	Home,
	/// The owner wrote it.
	Owner,
}

/// One word of a hearing, as the phone reads it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeardWord {
	pub text: String,
	/// When it began, in milliseconds from the take's start.
	pub start_ms: i64,
	/// When it ended, the same way.
	pub end_ms: i64,
	/// How sure the model was of it, 0 to 1; absent for the owner's words
	/// and a word heard before it was kept.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub p: Option<f64>,
	/// The language it was heard in, as Whisper's code; absent where none
	/// was kept.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub language: Option<String>,
}

impl From<Word> for HeardWord {
	fn from(word: Word) -> Self {
		Self {
			text: word.text,
			start_ms: word.start_ms,
			end_ms: word.end_ms,
			p: word.p,
			language: word.language,
		}
	}
}

/// One transcript of a take that ended: heard, or found silent.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hearing {
	/// The model it was heard with, or `owner:<n>` for the owner's.
	pub model: String,
	pub by: By,
	/// `heard` or `silent`.
	pub outcome: &'static str,
	/// For `heard`: the language, as Whisper's code.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub language: Option<String>,
	/// When it was heard, or when the owner saved it.
	pub heard_at: String,
	/// For `heard`: the text.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub text: Option<String>,
	/// For `heard`: its words in the order said; empty for a hearing from
	/// before words were kept.
	pub words: Vec<HeardWord>,
}

/// `GET /voice-notes/:id/transcripts`: every hearing of a take that ended.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hearings {
	/// The take's note id.
	pub note: String,
	/// The hearing a reader of the take's words takes: the owner's newest
	/// save, else the newest that heard something; none until one has.
	pub chosen: Option<String>,
	/// The owner's saves and home's hearings, newest first.
	pub hearings: Vec<Hearing>,
}

/// One word of the owner's, as the phone sends it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaidWord {
	text: String,
	start_ms: i64,
	end_ms: i64,
	#[serde(default)]
	language: Option<String>,
}

/// The body of `PUT /voice-notes/:id/transcripts/owner/:n`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaidWords {
	based_on: String,
	words: Vec<SaidWord>,
}

/// The owner's words for a take, checked: what a save keeps.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnerWords {
	/// The hearing it corrects, by model.
	pub based_on: String,
	pub words: Vec<Word>,
}

impl OwnerWords {
	/// The words as one text, a space between each.
	#[must_use]
	pub fn text(&self) -> String {
		let mut text = String::new();
		for word in &self.words {
			if !text.is_empty() {
				text.push(' ');
			}
			text.push_str(&word.text);
		}
		text
	}
}

/// Whether `code` is a language as Whisper writes one: two or three
/// lowercase letters, the rule `voice_transcript_word` holds.
fn is_language(code: &str) -> bool {
	(2..=3).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_lowercase())
}

/// The save number `n` names, if it names one.
#[must_use]
pub fn save_number(n: &str) -> Option<i64> {
	if n.is_empty() || n.len() > 4 || !n.bytes().all(|b| b.is_ascii_digit()) || n.starts_with('0') {
		return None;
	}
	n.parse().ok().filter(|n| (1..=MAX_SAVES).contains(n))
}

/// Reads and checks `body`, a save's words.
///
/// # Errors
/// Each refused field: `body` when it is not the words as JSON, `basedOn`
/// when it names no model, and `words` for none, too many, or a word that
/// is not one word, is timed out of order or names no language code. The
/// reasons are fixed words, never what was said.
pub fn read_words(body: &[u8]) -> Result<OwnerWords, Vec<Problem>> {
	let Ok(said) = serde_json::from_slice::<SaidWords>(body) else {
		return Err(vec![("body", "not the owner's words: { basedOn, words: [{ text, startMs, endMs, language? }] }")]);
	};
	let mut problems: Vec<Problem> = Vec::new();
	if said.based_on.trim().is_empty() || said.based_on.len() > 100 {
		problems.push(("basedOn", "not a hearing's model: 1 to 100 characters"));
	}
	if said.words.is_empty() {
		problems.push(("words", "none: a take's words hold at least one"));
	} else if said.words.len() > MAX_WORDS {
		problems.push(("words", "more than a take may hold (12,000)"));
	}
	let mut words = Vec::with_capacity(said.words.len());
	let mut previous_start = 0_i64;
	let (mut not_words, mut out_of_order, mut not_codes) = (false, false, false);
	for word in said.words {
		let text = word.text.trim();
		if text.is_empty() || text.chars().any(char::is_whitespace) || text.chars().count() > MAX_WORD_CHARS {
			not_words = true;
		}
		if word.start_ms < previous_start || word.end_ms < word.start_ms {
			out_of_order = true;
		}
		previous_start = word.start_ms.max(previous_start);
		if word.language.as_deref().is_some_and(|code| !is_language(code)) {
			not_codes = true;
		}
		words.push(Word {
			text: text.to_owned(),
			start_ms: word.start_ms,
			end_ms: word.end_ms,
			p: None,
			language: word.language,
		});
	}
	if not_words {
		problems.push(("words", "a word that is not one: some text, no spaces, at most 100 characters"));
	}
	if out_of_order {
		problems.push(("words", "times out of order: from the take's start, each ending after it begins, in the order said"));
	}
	if not_codes {
		problems.push(("words", "a language that is not Whisper's code (en, ko)"));
	}
	if problems.is_empty() {
		Ok(OwnerWords { based_on: said.based_on, words })
	} else {
		Err(problems)
	}
}

/// Why a save wrote nothing.
#[derive(Debug)]
pub enum Refused {
	/// The subject holds no Studio take under that id.
	NoTake,
	/// `basedOn` names no hearing of the take that heard something.
	NotHeard,
	/// The save number already holds other words, which are never replaced.
	Rewritten,
	Storage(VoiceError),
}

impl From<sqlx::Error> for Refused {
	fn from(err: sqlx::Error) -> Self {
		Self::Storage(VoiceError::Storage(err))
	}
}

impl From<VoiceError> for Refused {
	fn from(err: VoiceError) -> Self {
		Self::Storage(err)
	}
}

/// What [`Refused::Rewritten`] says.
pub const REWRITTEN: &str = "this save of the take already holds other words: the owner's transcripts are append-only";

/// What [`Refused::NotHeard`] says, for the `basedOn` field.
pub const NOT_HEARD: &str = "names no hearing of this take that heard something";

/// Whether `subject` holds a Studio take under `note_id`.
async fn holds_take(conn: &mut SqliteConnection, subject: &str, note_id: &str) -> Result<bool, sqlx::Error> {
	sqlx::query_scalar!(
		r#"SELECT EXISTS (SELECT 1 FROM voice_note WHERE subject_id = ?1 AND note_id = ?2 AND of_kind = 'studio') AS "held!: bool""#,
		subject,
		note_id
	)
	.fetch_one(&mut *conn)
	.await
}

/// The hearing [`Hearings::chosen`] names: the owner's newest save, else the
/// newest that heard something, as `transcript::words` orders them.
fn chosen(hearings: &[Hearing]) -> Option<String> {
	let heard = || hearings.iter().filter(|hearing| hearing.outcome == "heard");
	heard()
		.find(|hearing| hearing.by == By::Owner)
		.or_else(|| heard().next())
		.map(|hearing| hearing.model.clone())
}

/// Every hearing of `subject`'s Studio take `note_id` that ended, the
/// owner's saves included, newest first; `None` when the subject holds no
/// Studio take under that id.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn hearings(conn: &mut SqliteConnection, subject: &str, note_id: &str) -> Result<Option<Hearings>, sqlx::Error> {
	if !holds_take(conn, subject, note_id).await? {
		return Ok(None);
	}
	let rows = sqlx::query!(
		r#"SELECT model, engine, outcome, language, heard_at, text FROM voice_transcript
		 WHERE subject_id = ?1 AND note_id = ?2 AND outcome IN ('heard', 'silent')
		 ORDER BY heard_at DESC, model DESC"#,
		subject,
		note_id
	)
	.fetch_all(&mut *conn)
	.await?;
	let mut hearings = Vec::with_capacity(rows.len());
	for row in rows {
		let heard = row.outcome == "heard";
		let words = if heard {
			words_heard_by(conn, subject, note_id, &row.model).await?.into_iter().map(HeardWord::from).collect()
		} else {
			Vec::new()
		};
		hearings.push(Hearing {
			by: if row.engine == ENGINE { By::Owner } else { By::Home },
			model: row.model,
			outcome: if heard { "heard" } else { "silent" },
			language: row.language,
			heard_at: row.heard_at,
			text: row.text,
			words,
		});
	}
	Ok(Some(Hearings {
		note: note_id.to_owned(),
		chosen: chosen(&hearings),
		hearings,
	}))
}

/// Whether two lists of words say the same, time and language included.
fn same_words(held: &[Word], said: &[Word]) -> bool {
	held.len() == said.len()
		&& held
			.iter()
			.zip(said)
			.all(|(a, b)| a.text == b.text && a.start_ms == b.start_ms && a.end_ms == b.end_ms && a.language == b.language)
}

/// The owner's `n`th save of `subject`'s Studio take `note_id`, as of `now`:
/// stored, or, when that save already holds these very words, nothing.
///
/// Run it in a `BEGIN IMMEDIATE` transaction, so the reads that decide and
/// the writes see one snapshot.
///
/// # Errors
/// [`Refused::NoTake`] for no such take, [`Refused::NotHeard`] when
/// `basedOn` names no hearing of it that heard something,
/// [`Refused::Rewritten`] for other words under a held save, and
/// [`Refused::Storage`] for any `sqlx` failure. A refusal writes nothing.
pub async fn keep_save(conn: &mut SqliteConnection, subject: &str, note_id: &str, n: i64, words: &OwnerWords, now: &str) -> Result<(Change, Hearing), Refused> {
	if !holds_take(conn, subject, note_id).await? {
		return Err(Refused::NoTake);
	}
	let model = model(n);
	let held = sqlx::query!(
		r#"SELECT language AS "language!", heard_at, text AS "text!" FROM voice_transcript
		 WHERE subject_id = ?1 AND note_id = ?2 AND model = ?3 AND outcome = 'heard'"#,
		subject,
		note_id,
		model
	)
	.fetch_optional(&mut *conn)
	.await?;
	if let Some(held) = held {
		let held_words = words_heard_by(conn, subject, note_id, &model).await?;
		if !same_words(&held_words, &words.words) {
			return Err(Refused::Rewritten);
		}
		return Ok((
			Change::Unchanged,
			Hearing {
				model,
				by: By::Owner,
				outcome: "heard",
				language: Some(held.language),
				heard_at: held.heard_at,
				text: Some(held.text),
				words: held_words.into_iter().map(HeardWord::from).collect(),
			},
		));
	}
	let base = sqlx::query!(
		r#"SELECT language AS "language!", language_p AS "language_p!", duration_ms AS "duration_ms!" FROM voice_transcript
		 WHERE subject_id = ?1 AND note_id = ?2 AND model = ?3 AND outcome = 'heard'"#,
		subject,
		note_id,
		words.based_on
	)
	.fetch_optional(&mut *conn)
	.await?
	.ok_or(Refused::NotHeard)?;
	let attempt = Attempt {
		model: &model,
		engine: ENGINE,
		outcome: Outcome::Heard {
			language: base.language.clone(),
			language_p: base.language_p,
			text: words.text(),
			words: words.words.clone(),
		},
		duration_ms: Some(base.duration_ms),
		took_ms: 0,
	};
	match keep_attempt(conn, subject, note_id, &attempt, now).await? {
		Kept::Added(_) => Ok((
			Change::Added,
			Hearing {
				model,
				by: By::Owner,
				outcome: "heard",
				language: Some(base.language),
				heard_at: now.to_owned(),
				text: Some(words.text()),
				words: words.words.iter().cloned().map(HeardWord::from).collect(),
			},
		)),
		// Read above in this transaction: neither can happen here.
		Kept::Ended => Err(Refused::Rewritten),
		Kept::NoNote => Err(Refused::NoTake),
	}
}

#[cfg(test)]
mod tests {
	use super::{hearings, keep_save, model, read_words, save_number, By, Refused};
	use crate::transcript::{keep_attempt, words, Attempt, Outcome, Word};
	use crate::{describe, put_note, Change, Said, SaidBite};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";
	const B: &str = "subject-b";
	const SMALL: &str = "ggml-small-q5_1.bin";
	const MEDIUM: &str = "ggml-medium-q5_0.bin";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	async fn note(pool: &SqlitePool, subject: &str, id: &str, of: &str) {
		let said = Said {
			of: Some(of),
			of_ref: Some("build-journal-ep04:s3:2"),
			at: Some("2026-10-30T18:20:00Z"),
			content_type: Some("audio/mp4"),
			bite: SaidBite::default(),
		};
		let mut tx = pool.begin().await.unwrap();
		put_note(&mut tx, subject, &describe(id, said, b"take").unwrap(), b"take", "2026-10-30T18:21:00.000Z")
			.await
			.unwrap();
		tx.commit().await.unwrap();
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

	async fn hear(pool: &SqlitePool, subject: &str, id: &str, model: &str, text: &str, at: &str) {
		let attempt = Attempt {
			model,
			engine: "whisper.cpp 1.8.3",
			outcome: Outcome::Heard {
				language: String::from("en"),
				language_p: 0.97,
				text: text.to_owned(),
				words: said(text),
			},
			duration_ms: Some(4_000),
			took_ms: 2_000,
		};
		let mut tx = pool.begin().await.unwrap();
		keep_attempt(&mut tx, subject, id, &attempt, at).await.unwrap();
		tx.commit().await.unwrap();
	}

	/// The owner's words, as the phone sends them: `text`'s words at the
	/// times `said` gives them, on `based_on`.
	fn body(based_on: &str, text: &str) -> Vec<u8> {
		let words: Vec<serde_json::Value> = said(text)
			.into_iter()
			.map(|word| serde_json::json!({ "text": word.text, "startMs": word.start_ms, "endMs": word.end_ms, "language": "en" }))
			.collect();
		serde_json::json!({ "basedOn": based_on, "words": words }).to_string().into_bytes()
	}

	async fn save(pool: &SqlitePool, subject: &str, id: &str, n: i64, body: &[u8], at: &str) -> Result<Change, Refused> {
		let words = read_words(body).unwrap();
		let mut tx = pool.begin().await.unwrap();
		let kept = keep_save(&mut tx, subject, id, n, &words, at).await;
		tx.commit().await.unwrap();
		kept.map(|(change, _)| change)
	}

	async fn words_of(pool: &SqlitePool, subject: &str, id: &str) -> Vec<String> {
		let mut conn = pool.acquire().await.unwrap();
		words(&mut conn, subject, id).await.unwrap().into_iter().map(|word| word.text).collect()
	}

	#[test]
	fn a_save_is_numbered_from_one() {
		assert_eq!(save_number("1"), Some(1));
		assert_eq!(save_number("9999"), Some(9_999));
		for refused in ["", "0", "01", "10000", "-1", "1a", "+1", " 1"] {
			assert_eq!(save_number(refused), None, "{refused:?}");
		}
		assert_eq!(model(3), "owner:3");
	}

	/// What a save may hold, each refusal naming its field in fixed words.
	#[test]
	fn a_save_holds_one_word_at_a_time_in_the_order_said() {
		let fields = |body: &str| -> Vec<&'static str> { read_words(body.as_bytes()).unwrap_err().into_iter().map(|(field, _)| field).collect() };
		assert_eq!(fields("not json"), ["body"]);
		assert_eq!(fields(r#"{"basedOn":"m","words":[],"p":1}"#), ["body"], "nothing else is taken");
		assert_eq!(
			fields(r#"{"basedOn":"m","words":[{"text":"a","startMs":0,"endMs":1,"p":0.5}]}"#),
			["body"],
			"the owner's words carry no p"
		);
		assert_eq!(fields(r#"{"basedOn":" ","words":[]}"#), ["basedOn", "words"]);
		assert_eq!(fields(r#"{"basedOn":"m","words":[{"text":"two words","startMs":0,"endMs":1}]}"#), ["words"]);
		assert_eq!(fields(r#"{"basedOn":"m","words":[{"text":" ","startMs":0,"endMs":1}]}"#), ["words"]);
		assert_eq!(
			fields(r#"{"basedOn":"m","words":[{"text":"late","startMs":500,"endMs":900},{"text":"early","startMs":100,"endMs":300}]}"#),
			["words"]
		);
		assert_eq!(fields(r#"{"basedOn":"m","words":[{"text":"x","startMs":500,"endMs":400}]}"#), ["words"]);
		assert_eq!(fields(r#"{"basedOn":"m","words":[{"text":"x","startMs":-1,"endMs":400}]}"#), ["words"]);
		assert_eq!(fields(r#"{"basedOn":"m","words":[{"text":"x","startMs":0,"endMs":400,"language":"Korean"}]}"#), ["words"]);
		let many = serde_json::json!({ "basedOn": "m", "words": vec![serde_json::json!({ "text": "a", "startMs": 0, "endMs": 1 }); super::MAX_WORDS + 1] });
		assert_eq!(fields(&many.to_string()), ["words"]);

		let read = read_words(r#"{"basedOn":"m","words":[{"text":" 은 ","startMs":0,"endMs":400,"language":"ko"},{"text":"again","startMs":0,"endMs":800}]}"#.as_bytes()).unwrap();
		assert_eq!(read.text(), "은 again", "a word is kept trimmed, and two may start together");
		assert_eq!(read.words[0].p, None);
	}

	/// The round: home hears a take, the owner fixes a word, a replay changes
	/// nothing, other words under that save are refused, a later fix is a
	/// new save, and the engines' transcripts are never touched.
	#[tokio::test]
	async fn the_owners_words_are_kept_beside_the_engines_and_never_rewritten() {
		let pool = pool().await;
		note(&pool, A, "t1", "studio").await;
		hear(&pool, A, "t1", SMALL, "the particle and again", "2026-10-30T18:30:00.000Z").await;

		let fixed = body(SMALL, "the particle 은 again");
		assert_eq!(save(&pool, A, "t1", 1, &fixed, "2026-10-30T19:00:00.000Z").await.unwrap(), Change::Added);
		assert_eq!(save(&pool, A, "t1", 1, &fixed, "2026-10-30T19:05:00.000Z").await.unwrap(), Change::Unchanged);
		assert!(matches!(
			save(&pool, A, "t1", 1, &body(SMALL, "something else"), "2026-10-30T19:06:00.000Z").await,
			Err(Refused::Rewritten)
		));
		assert_eq!(words_of(&pool, A, "t1").await, ["the", "particle", "은", "again"], "the owner's words are read");
		assert_eq!(
			save(&pool, A, "t1", 2, &body(&model(1), "the particle 은 again!"), "2026-10-30T19:10:00.000Z")
				.await
				.unwrap(),
			Change::Added
		);

		let mut conn = pool.acquire().await.unwrap();
		let read = hearings(&mut conn, A, "t1").await.unwrap().unwrap();
		drop(conn);
		assert_eq!(read.chosen.as_deref(), Some("owner:2"));
		let listed: Vec<(&str, By, Option<&str>)> = read.hearings.iter().map(|h| (h.model.as_str(), h.by, h.text.as_deref())).collect();
		assert_eq!(
			listed,
			[
				("owner:2", By::Owner, Some("the particle 은 again!")),
				("owner:1", By::Owner, Some("the particle 은 again")),
				(SMALL, By::Home, Some("the particle and again")),
			],
			"every save is kept, newest first, and home's hearing is as it heard it"
		);
		assert_eq!(read.hearings[2].words[2].p, Some(0.9), "home's words keep how sure it was");
		assert_eq!(read.hearings[0].words[2].p, None, "the owner's carry no p");

		let engines: Vec<(String, String, Option<i64>)> = sqlx::query_as("SELECT model, engine, duration_ms FROM voice_transcript ORDER BY heard_at")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(
			engines,
			[
				(String::from(SMALL), String::from("whisper.cpp 1.8.3"), Some(4_000)),
				(String::from("owner:1"), String::from("owner"), Some(4_000)),
				(String::from("owner:2"), String::from("owner"), Some(4_000)),
			],
			"the owner is credited as the engine, and the take's length is the hearing's"
		);
	}

	/// The owner's words stay the truth when a better model hears the take
	/// later: a reader still takes the owner's.
	#[tokio::test]
	async fn the_owners_words_outrank_a_later_model() {
		let pool = pool().await;
		note(&pool, A, "t1", "studio").await;
		hear(&pool, A, "t1", SMALL, "hello their", "2026-10-30T18:30:00.000Z").await;
		save(&pool, A, "t1", 1, &body(SMALL, "hello there"), "2026-10-30T19:00:00.000Z").await.unwrap();
		hear(&pool, A, "t1", MEDIUM, "hello, there", "2026-11-02T09:00:00.000Z").await;

		assert_eq!(words_of(&pool, A, "t1").await, ["hello", "there"]);
		let mut conn = pool.acquire().await.unwrap();
		let read = hearings(&mut conn, A, "t1").await.unwrap().unwrap();
		assert_eq!(read.chosen.as_deref(), Some("owner:1"));
		assert_eq!(read.hearings[0].model, MEDIUM, "listed newest first all the same");
	}

	/// Only a Studio take the subject holds, and only on a hearing that heard
	/// it; a refusal writes nothing.
	#[tokio::test]
	async fn a_save_needs_a_take_the_subject_holds_and_a_hearing_of_it() {
		let pool = pool().await;
		note(&pool, A, "t1", "studio").await;
		note(&pool, A, "r1", "reflection").await;
		hear(&pool, A, "r1", SMALL, "a reflection", "2026-10-30T18:30:00.000Z").await;
		let fixed = body(SMALL, "hello there");

		assert!(matches!(save(&pool, A, "t1", 1, &fixed, "now").await, Err(Refused::NotHeard)), "home has not heard it yet");
		assert!(matches!(save(&pool, A, "r1", 1, &fixed, "now").await, Err(Refused::NoTake)), "a reflection is no take");
		assert!(matches!(save(&pool, B, "t1", 1, &fixed, "now").await, Err(Refused::NoTake)), "A's take is not B's");
		hear(&pool, A, "t1", SMALL, "hello their", "2026-10-30T18:30:00.000Z").await;
		assert!(matches!(save(&pool, A, "t1", 1, &body(MEDIUM, "x"), "now").await, Err(Refused::NotHeard)));

		let owners: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voice_transcript WHERE engine = 'owner'")
			.fetch_one(&pool)
			.await
			.unwrap();
		assert_eq!(owners, 0, "nothing refused was written");
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(hearings(&mut conn, A, "r1").await.unwrap(), None, "a reflection's words are read by no route");
		assert_eq!(hearings(&mut conn, B, "t1").await.unwrap(), None);
		let read = hearings(&mut conn, A, "t1").await.unwrap().unwrap();
		assert_eq!(read.chosen.as_deref(), Some(SMALL));
	}

	/// A take found silent is listed so, with no words and nothing chosen.
	#[tokio::test]
	async fn a_silent_take_is_listed_with_nothing_chosen() {
		let pool = pool().await;
		note(&pool, A, "t1", "studio").await;
		let mut conn = pool.acquire().await.unwrap();
		let none = hearings(&mut conn, A, "t1").await.unwrap().unwrap();
		assert_eq!((none.chosen, none.hearings.len()), (None, 0), "not heard yet");
		let silent = Attempt {
			model: SMALL,
			engine: "whisper.cpp 1.8.3",
			outcome: Outcome::Silent,
			duration_ms: Some(3_000),
			took_ms: 900,
		};
		keep_attempt(&mut conn, A, "t1", &silent, "2026-10-30T18:30:00.000Z").await.unwrap();
		let read = hearings(&mut conn, A, "t1").await.unwrap().unwrap();
		assert_eq!(read.chosen, None);
		assert_eq!((read.hearings[0].outcome, read.hearings[0].words.len()), ("silent", 0));
	}
}
