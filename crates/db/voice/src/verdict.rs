//! What home made of each transcript it keeps (paulgsc/server#908, HEARD1):
//! `voice_transcript_verdict`.
//!
//! `20261116000200_create_voice_transcript_verdict.up.sql` records the
//! design; in short:
//!
//! - **Four verdicts.** [`Kind::Heard`], [`Kind::Garbled`] (the transcriber's
//!   own signals say what it heard is not what was said),
//!   [`Kind::NothingSaid`] and [`Kind::Failed`]. `file_host`'s
//!   `transcribe::verdict` judges; this module only keeps and reads.
//! - **Append-only, beside the attempt.** [`keep`] writes a verdict in the
//!   transaction that keeps its attempt, named by the attempt's key and the
//!   rule that judged it. Nothing is updated.
//! - **Only final attempts.** An ending (`heard`, `silent`) and the last
//!   failure the worker makes on a note. A failure that will be retried is
//!   not judged, so no read calls a note failed while it may yet be heard.
//! - **Every read that returns a transcript says it** ([`of_note`],
//!   [`of_ending`]): the verdict of the transcript that read chose, or, for a
//!   note no attempt ended, its final failure. A transcript kept before
//!   verdicts were has none, and a reader gets none.

use crate::VoiceError;
use serde::Serialize;
use sqlx::SqliteConnection;

/// What home made of a transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
	/// What was heard is what was said, as far as the signals tell.
	Heard,
	/// Speech was heard, and the transcriber's own signals say its words are
	/// not to be trusted: a reader treats the take as unanswered.
	Garbled,
	/// No speech was found.
	NothingSaid,
	/// Every attempt failed: the take was never heard.
	Failed,
}

impl Kind {
	/// The stored name, which is also all a log line or a metric may carry.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Heard => "heard",
			Self::Garbled => "garbled",
			Self::NothingSaid => "nothing_said",
			Self::Failed => "failed",
		}
	}

	/// Every kind, in [`Self::as_str`]'s order.
	pub const ALL: [Self; 4] = [Self::Heard, Self::Garbled, Self::NothingSaid, Self::Failed];

	/// The kind stored as `stored`, if this build knows it.
	pub(crate) fn parse(stored: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|kind| kind.as_str() == stored)
	}

	/// Whether a reader treats the take as unanswered: home could not hear it.
	#[must_use]
	pub const fn unheard(self) -> bool {
		matches!(self, Self::Garbled | Self::Failed)
	}

	/// Whether an essay may call the take a silence: home could not hear it,
	/// or heard nothing said in it.
	#[must_use]
	pub const fn may_be_silent(self) -> bool {
		!matches!(self, Self::Heard)
	}
}

/// What a heard transcript was judged on: the transcriber's own signals.
/// Serialized in `camelCase`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Signals {
	/// The mean of the segments' mean token log-probability, weighted by each
	/// segment's length: Whisper's `avg_logprob`, over the take.
	pub avg_logprob: f64,
	/// The most repetitive segment's text, as its length over its zlib
	/// length: Whisper's `compression_ratio`.
	pub compression_ratio: f64,
	/// The highest segment's probability that it held no speech: Whisper's
	/// `no_speech_prob`.
	pub no_speech_prob: f64,
	/// The share of the words heard that the model gave a probability below
	/// one half (`transcribe::verdict::WORD_P_FLOOR`), 0 to 1.
	pub unsure_share: f64,
	/// The language detected for longest, and that detection's probability.
	pub language: String,
	pub language_p: f64,
	/// The language the take's kind is said in, when it has one (`en`).
	pub expected_language: Option<String>,
	/// Words heard per second of the speech the VAD found.
	pub words_per_second: f64,
	/// How long that speech is.
	pub speech_ms: i64,
	/// How loud that speech is at its loudest sample and on average (RMS),
	/// in dB below full scale: quiet audio is what a garbled take most often
	/// is.
	pub peak_dbfs: f64,
	pub rms_dbfs: f64,
}

/// A transcript's verdict, as every read of a transcript carries it.
/// Serialized in `camelCase`, as `"verdict"`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Verdict {
	pub kind: Kind,
	/// Why: for [`Kind::Garbled`] any of `low_confidence`, `repetitive`,
	/// `no_speech`, `off_language`, `sparse`; for [`Kind::Failed`] one of
	/// `unreadable`, `too_long`, `deadline`, `engine`; empty otherwise.
	pub reasons: Vec<String>,
	/// Which thresholds judged it (`heard-1`).
	pub rule: String,
	/// What it was judged on: for [`Kind::Heard`] and [`Kind::Garbled`]
	/// only.
	pub signals: Option<Signals>,
}

/// The language a note said for `of_kind`, under `of_ref`, is said in, when
/// its source has one.
///
/// One arm per kind, and no wildcard, so a new kind says its language before
/// it compiles. A Rust Rounds answer and an interview take are English, and so
/// is a Studio take of a script's scene (`<project>:<shot>:<take>`) or of a
/// spoken card's part (`spoken:<card>.<part>`): each is read or said for an
/// English video (the owner, 2026-11-21, paulgsc/some-ui#3339, after a spoken
/// card's take was heard as German). Every other kind, and a Studio idea's
/// answer or a take with no scene, may mix languages (a reflection's Korean),
/// so has none. With a language, the engine hears the whole take in it, and a
/// verdict flags `off_language` when the take says otherwise.
#[must_use]
pub fn expected_language(of_kind: &str, of_ref: &str) -> Option<&'static str> {
	let of = crate::Of::named(of_kind)?;
	let english = match of {
		crate::Of::RustRound | crate::Of::Interview => true,
		crate::Of::Studio => crate::about::studio_script_or_spoken(of_ref),
		crate::Of::Reflection | crate::Of::Jobs | crate::Of::Scene | crate::Of::Game | crate::Of::Soundbite | crate::Of::Lens | crate::Of::Leetype => false,
	};
	english.then_some("en")
}

/// One attempt, as [`keep`] names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Judged<'a> {
	pub subject: &'a str,
	pub note_id: &'a str,
	pub model: &'a str,
	pub attempt: i64,
}

/// Stores `verdict` on the attempt `judged`, as of `now`, in the transaction
/// that keeps the attempt.
///
/// # Errors
/// Any `sqlx` failure; a verdict the rule already gave this attempt is one.
pub async fn keep(conn: &mut SqliteConnection, judged: Judged<'_>, verdict: &Verdict, now: &str) -> Result<(), VoiceError> {
	let kind = verdict.kind.as_str();
	let reasons = verdict.reasons.join(" ");
	let signals = verdict.signals.as_ref();
	// A number the schema would hold is kept as it is; one that is not a
	// number is kept as none, so a NaN never fails the attempt's write.
	let finite = |value: f64| value.is_finite().then_some(value);
	let avg_logprob = signals.and_then(|s| finite(s.avg_logprob));
	let compression_ratio = signals.and_then(|s| finite(s.compression_ratio));
	let no_speech_prob = signals.and_then(|s| finite(s.no_speech_prob));
	let unsure_share = signals.and_then(|s| finite(s.unsure_share));
	let language = signals.map(|s| s.language.as_str());
	let language_p = signals.and_then(|s| finite(s.language_p));
	let expected_language = signals.and_then(|s| s.expected_language.as_deref());
	let words_per_second = signals.and_then(|s| finite(s.words_per_second));
	let speech_ms = signals.map(|s| s.speech_ms.max(0));
	let peak_dbfs = signals.and_then(|s| finite(s.peak_dbfs));
	let rms_dbfs = signals.and_then(|s| finite(s.rms_dbfs));
	sqlx::query!(
		"INSERT INTO voice_transcript_verdict
		   (subject_id, note_id, model, attempt, rule, verdict, reasons, avg_logprob, compression_ratio, no_speech_prob, unsure_share,
		    language, language_p, expected_language, words_per_second, speech_ms, peak_dbfs, rms_dbfs, judged_at)
		 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
		judged.subject,
		judged.note_id,
		judged.model,
		judged.attempt,
		verdict.rule,
		kind,
		reasons,
		avg_logprob,
		compression_ratio,
		no_speech_prob,
		unsure_share,
		language,
		language_p,
		expected_language,
		words_per_second,
		speech_ms,
		peak_dbfs,
		rms_dbfs,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok(())
}

/// One stored row, as [`read`] reads it back.
pub(crate) struct Row {
	pub(crate) rule: String,
	pub(crate) verdict: String,
	pub(crate) reasons: String,
	pub(crate) avg_logprob: Option<f64>,
	pub(crate) compression_ratio: Option<f64>,
	pub(crate) no_speech_prob: Option<f64>,
	pub(crate) unsure_share: Option<f64>,
	pub(crate) language: Option<String>,
	pub(crate) language_p: Option<f64>,
	pub(crate) expected_language: Option<String>,
	pub(crate) words_per_second: Option<f64>,
	pub(crate) speech_ms: Option<i64>,
	pub(crate) peak_dbfs: Option<f64>,
	pub(crate) rms_dbfs: Option<f64>,
}

pub(crate) fn read(row: Row) -> Result<Verdict, VoiceError> {
	let kind = Kind::parse(&row.verdict).ok_or(VoiceError::Unreadable)?;
	let signals = match row.speech_ms {
		Some(speech_ms) => Some(Signals {
			avg_logprob: row.avg_logprob.unwrap_or(f64::NAN),
			compression_ratio: row.compression_ratio.unwrap_or(f64::NAN),
			no_speech_prob: row.no_speech_prob.unwrap_or(f64::NAN),
			unsure_share: row.unsure_share.unwrap_or(f64::NAN),
			language: row.language.unwrap_or_default(),
			language_p: row.language_p.unwrap_or(f64::NAN),
			expected_language: row.expected_language,
			words_per_second: row.words_per_second.unwrap_or(f64::NAN),
			speech_ms,
			peak_dbfs: row.peak_dbfs.unwrap_or(f64::NAN),
			rms_dbfs: row.rms_dbfs.unwrap_or(f64::NAN),
		}),
		None => None,
	};
	Ok(Verdict {
		kind,
		reasons: row.reasons.split_whitespace().map(str::to_owned).collect(),
		rule: row.rule,
		signals,
	})
}

/// The verdict of `subject`'s note `note_id`, as every read of its
/// transcript carries it.
///
/// The verdict of the transcript `crate::transcript::heard` chooses (the
/// newest that heard something, else the newest silent ending), by the
/// newest rule; none when that transcript was kept before verdicts were.
/// For a note no attempt ended, its newest final failure; none while it may
/// yet be heard by the model that failed. A model a deployment swapped in
/// later hears the note afresh, and its ending then replaces the failure
/// here (and, filed at a higher number, in the mailbox too).
///
/// # Errors
/// Any `sqlx` failure, or a stored verdict this build does not know.
pub async fn of_note(conn: &mut SqliteConnection, subject: &str, note_id: &str) -> Result<Option<Verdict>, VoiceError> {
	Ok(newest(conn, subject, note_id).await?.map(|newest| newest.verdict))
}

/// A note's verdict, as [`of_note`] chooses it, with the key it is kept
/// under: what an act of the owner's on it answers (HEARD3,
/// [`crate::attention`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Newest {
	/// The attempt judged: the model and its attempt.
	pub model: String,
	pub attempt: i64,
	pub verdict: Verdict,
}

/// [`of_note`]'s verdict, with its key.
///
/// # Errors
/// Any `sqlx` failure, or a stored verdict this build does not know.
pub async fn newest(conn: &mut SqliteConnection, subject: &str, note_id: &str) -> Result<Option<Newest>, VoiceError> {
	// The ending is `transcript::heard`'s choice, by its ORDER BY; then that
	// attempt's verdict, if it has one. Only with no ending at all, the
	// newest failure judged final.
	let row = sqlx::query!(
		r#"WITH ending AS (
		     SELECT t.model, t.attempt FROM voice_transcript t
		     WHERE t.subject_id = ?1 AND t.note_id = ?2 AND t.outcome IN ('heard', 'silent')
		       AND NOT EXISTS (SELECT 1 FROM voice_note_superseded sup WHERE sup.subject_id = t.subject_id AND sup.note_id = t.note_id AND sup.model = t.model)
		     ORDER BY t.outcome = 'heard' DESC, t.heard_at DESC, t.model DESC
		     LIMIT 1)
		   SELECT v.model AS "model!", v.attempt AS "attempt!", v.rule AS "rule!", v.verdict AS "verdict!", v.reasons AS "reasons!",
		          v.avg_logprob, v.compression_ratio, v.no_speech_prob, v.unsure_share, v.language, v.language_p, v.expected_language,
		          v.words_per_second, v.speech_ms, v.peak_dbfs, v.rms_dbfs
		   FROM voice_transcript_verdict v
		   WHERE v.subject_id = ?1 AND v.note_id = ?2
		     AND (EXISTS (SELECT 1 FROM ending e WHERE e.model = v.model AND e.attempt = v.attempt)
		          OR (NOT EXISTS (SELECT 1 FROM ending) AND v.verdict = 'failed'
		              AND NOT EXISTS (SELECT 1 FROM voice_note_superseded sup WHERE sup.subject_id = v.subject_id AND sup.note_id = v.note_id AND sup.model = v.model)))
		   ORDER BY v.judged_at DESC, v.rule DESC, v.model DESC, v.attempt DESC
		   LIMIT 1"#,
		subject,
		note_id
	)
	.fetch_optional(&mut *conn)
	.await?;
	let Some(row) = row else {
		return Ok(None);
	};
	let verdict = read(Row {
		rule: row.rule,
		verdict: row.verdict,
		reasons: row.reasons,
		avg_logprob: row.avg_logprob,
		compression_ratio: row.compression_ratio,
		no_speech_prob: row.no_speech_prob,
		unsure_share: row.unsure_share,
		language: row.language,
		language_p: row.language_p,
		expected_language: row.expected_language,
		words_per_second: row.words_per_second,
		speech_ms: row.speech_ms,
		peak_dbfs: row.peak_dbfs,
		rms_dbfs: row.rms_dbfs,
	})?;
	Ok(Some(Newest {
		model: row.model,
		attempt: row.attempt,
		verdict,
	}))
}

/// The verdict of what `model` made of `subject`'s note `note_id`.
///
/// That of the transcript it ended the note with, or of its last failure on
/// the note when it ended it with none, by the newest rule: what a
/// transcript filed under that model carries. None when it was kept before
/// verdicts were.
///
/// # Errors
/// Any `sqlx` failure, or a stored verdict this build does not know.
pub async fn of_ending(conn: &mut SqliteConnection, subject: &str, note_id: &str, model: &str) -> Result<Option<Verdict>, VoiceError> {
	let row = sqlx::query_as!(
		Row,
		r#"SELECT v.rule AS "rule!", v.verdict AS "verdict!", v.reasons AS "reasons!", v.avg_logprob, v.compression_ratio, v.no_speech_prob,
		          v.unsure_share, v.language, v.language_p, v.expected_language, v.words_per_second, v.speech_ms, v.peak_dbfs, v.rms_dbfs
		   FROM voice_transcript t
		   JOIN voice_transcript_verdict v
		     ON v.subject_id = t.subject_id AND v.note_id = t.note_id AND v.model = t.model AND v.attempt = t.attempt
		   WHERE t.subject_id = ?1 AND t.note_id = ?2 AND t.model = ?3
		     AND NOT EXISTS (SELECT 1 FROM voice_note_superseded sup WHERE sup.subject_id = t.subject_id AND sup.note_id = t.note_id AND sup.model = t.model)
		   ORDER BY t.outcome IN ('heard', 'silent') DESC, t.attempt DESC, v.judged_at DESC, v.rule DESC
		   LIMIT 1"#,
		subject,
		note_id,
		model
	)
	.fetch_optional(&mut *conn)
	.await?;
	row.map(read).transpose()
}

/// What a week's routine made of `subject`'s take `note_id` (HEARD1).
///
/// The verdict of the transcript the routine reads, as an essay is checked
/// against it, so home's check and the routine judge the same one.
///
/// The routine reads the take's transcripts the mailbox took (handed over
/// `added` or `unchanged`: one filed and not yet taken, or refused, the
/// routine never saw), and judges by
/// the winning one's verdict alone (paulgsc/some-ui#3073's `heardByNote`):
/// with `verbatim_first` (an interview take) one whose model holds
/// `+verbatim` before any other, then the highest number filed; a Rust
/// Rounds take by the highest number alone. None when nothing was filed (the
/// routine waits) or the winner carries no verdict (the routine reads it as
/// heard).
///
/// # Errors
/// Any `sqlx` failure, or a stored verdict this build does not know.
pub async fn as_the_routine_reads(conn: &mut SqliteConnection, subject: &str, note_id: &str, verbatim_first: bool) -> Result<Option<Kind>, VoiceError> {
	let filed = sqlx::query_scalar!(
		r#"SELECT filed.model AS "model!" FROM mailbox_transcript AS filed
		   WHERE filed.subject_id = ?1 AND filed.note_id = ?2
		     AND EXISTS (
		       SELECT 1 FROM mailbox_handed AS handed
		       WHERE handed.subject_id = filed.subject_id AND handed.folder = 'transcript'
		         AND handed.name = 'transcript-' || filed.note_id || '-' || filed.n || '.json'
		         AND handed.answer IN ('added', 'unchanged'))
		   ORDER BY (?3 AND instr(filed.model, '+verbatim') > 0) DESC, filed.n DESC
		   LIMIT 1"#,
		subject,
		note_id,
		verbatim_first
	)
	.fetch_optional(&mut *conn)
	.await?;
	let Some(model) = filed else {
		return Ok(None);
	};
	// The verdict of that model's last final attempt, read from the verdicts
	// alone, so it outlives the note's transcripts: a take the owner deleted
	// still reads as what home made of it (HEARD3), and an essay that called
	// it a silence stands. A model judges at most its ending, which is its
	// last attempt, or its last failure, so this is [`of_ending`]'s choice.
	let kind = sqlx::query_scalar!(
		r#"SELECT v.verdict AS "verdict!" FROM voice_transcript_verdict v
		   WHERE v.subject_id = ?1 AND v.note_id = ?2 AND v.model = ?3
		   ORDER BY v.attempt DESC, v.judged_at DESC, v.rule DESC
		   LIMIT 1"#,
		subject,
		note_id,
		model
	)
	.fetch_optional(&mut *conn)
	.await?;
	kind.map(|kind| Kind::parse(&kind).ok_or(VoiceError::Unreadable)).transpose()
}

/// Whether a week's routine can read any transcript of `subject`'s take.
///
/// One of `note_id`'s filed and taken by the mailbox, as
/// [`as_the_routine_reads`] chooses among. With none, the routine waits on
/// the take (paulgsc/some-ui's `monday` prepare: "home hasn't heard the take
/// yet"), so the week's essay is not written. One indexed read.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn routine_reads_any(conn: &mut SqliteConnection, subject: &str, note_id: &str) -> Result<bool, VoiceError> {
	let any = sqlx::query_scalar!(
		r#"SELECT EXISTS (
		     SELECT 1 FROM mailbox_transcript AS filed
		     WHERE filed.subject_id = ?1 AND filed.note_id = ?2
		       AND EXISTS (
		         SELECT 1 FROM mailbox_handed AS handed
		         WHERE handed.subject_id = filed.subject_id AND handed.folder = 'transcript'
		           AND handed.name = 'transcript-' || filed.note_id || '-' || filed.n || '.json'
		           AND handed.answer IN ('added', 'unchanged'))
		   ) AS "any!: bool""#,
		subject,
		note_id
	)
	.fetch_one(&mut *conn)
	.await?;
	Ok(any)
}

#[cfg(test)]
mod tests {
	use super::{expected_language, keep, of_ending, of_note, Judged, Kind, Signals, Verdict};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, x'00')")
			.bind(A)
			.execute(&pool)
			.await
			.unwrap();
		sqlx::query(
			"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES (?1, 'n', 'rust-round', '2026-w46:QM-311', 'then', 'audio/webm', 1, 's', 'now')",
		)
		.bind(A)
		.execute(&pool)
		.await
		.unwrap();
		pool
	}

	async fn attempt(pool: &SqlitePool, model: &str, attempt: i64, outcome: &str, heard_at: &str) {
		let heard = outcome == "heard";
		sqlx::query(
			"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, language, language_p, duration_ms, text, took_ms, heard_at)
			 VALUES (?1, 'n', ?2, 'e', ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9)",
		)
		.bind(A)
		.bind(model)
		.bind(attempt)
		.bind(outcome)
		.bind(heard.then_some("en"))
		.bind(heard.then_some(0.9))
		.bind((outcome == "heard" || outcome == "silent").then_some(1000))
		.bind(heard.then_some("words"))
		.bind(heard_at)
		.execute(pool)
		.await
		.unwrap();
	}

	/// `model`'s transcript filed as number `n`, and the mailbox's `answer`
	/// to it.
	async fn filed(pool: &SqlitePool, model: &str, n: i64, answer: &str) {
		sqlx::query("INSERT INTO mailbox_transcript (subject_id, note_id, model, n, filed_at) VALUES (?1, 'n', ?2, ?3, 'now')")
			.bind(A)
			.bind(model)
			.bind(n)
			.execute(pool)
			.await
			.unwrap();
		sqlx::query("INSERT INTO mailbox_handed (subject_id, folder, name, answer, handed_at) VALUES (?1, 'transcript', ?2, ?3, 'now')")
			.bind(A)
			.bind(String::from("transcript-n-") + &n.to_string() + ".json")
			.bind(answer)
			.execute(pool)
			.await
			.unwrap();
	}

	fn signals() -> Signals {
		Signals {
			avg_logprob: -0.3,
			compression_ratio: 1.4,
			no_speech_prob: 0.01,
			unsure_share: 0.0,
			language: String::from("en"),
			language_p: 0.97,
			expected_language: Some(String::from("en")),
			words_per_second: 2.2,
			speech_ms: 23_000,
			peak_dbfs: -6.0,
			rms_dbfs: -28.0,
		}
	}

	fn verdict(kind: Kind, reasons: &[&str], signals: Option<Signals>) -> Verdict {
		Verdict {
			kind,
			reasons: reasons.iter().map(|&reason| reason.to_owned()).collect(),
			rule: String::from("heard-1"),
			signals,
		}
	}

	async fn judge(pool: &SqlitePool, model: &str, attempt: i64, verdict: &Verdict, now: &str) {
		let mut conn = pool.acquire().await.unwrap();
		let judged = Judged {
			subject: A,
			note_id: "n",
			model,
			attempt,
		};
		keep(&mut conn, judged, verdict, now).await.unwrap();
	}

	#[test]
	fn a_take_is_english_when_its_source_is_and_a_reflection_never_is() {
		assert_eq!(expected_language("rust-round", "2026-w47:QM-311"), Some("en"));
		assert_eq!(expected_language("interview", "2026-w47:about-you"), Some("en"));
		assert_eq!(expected_language("studio", "build-journal-ep04:s3:2"), Some("en"), "a script's scene");
		assert_eq!(expected_language("studio", "spoken:sc1a2b3c.0"), Some("en"), "a spoken card's part");
		assert_eq!(expected_language("studio", "idea:i1:q2"), None, "an idea's answer");
		assert_eq!(expected_language("studio", "loose"), None, "a take with no scene");
		assert_eq!(expected_language("reflection", "session-7f2a"), None, "a reflection mixes Korean in");
		assert_eq!(expected_language("soundbite", "direct"), None);
		assert_eq!(expected_language("no-such-kind", "x"), None);
		assert!(Kind::Garbled.unheard() && Kind::Failed.unheard());
		assert!(!Kind::Heard.unheard() && !Kind::NothingSaid.unheard());
	}

	#[test]
	fn a_verdict_reads_as_the_shape_the_routines_code_against() {
		let json = serde_json::to_value(verdict(Kind::Garbled, &["low_confidence", "sparse"], Some(signals()))).unwrap();
		assert_eq!(
			json,
			serde_json::json!({
				"kind": "garbled",
				"reasons": ["low_confidence", "sparse"],
				"rule": "heard-1",
				"signals": {
					"avgLogprob": -0.3, "compressionRatio": 1.4, "noSpeechProb": 0.01, "unsureShare": 0.0,
					"language": "en", "languageP": 0.97, "expectedLanguage": "en",
					"wordsPerSecond": 2.2, "speechMs": 23000, "peakDbfs": -6.0, "rmsDbfs": -28.0
				}
			})
		);
		let failed = serde_json::to_value(verdict(Kind::NothingSaid, &[], None)).unwrap();
		assert_eq!(failed, serde_json::json!({ "kind": "nothing_said", "reasons": [], "rule": "heard-1", "signals": null }));
	}

	/// A verdict reads back as kept, on the ending the transcript read
	/// chooses; one kept before verdicts reads as none.
	#[tokio::test]
	async fn the_verdict_is_the_chosen_endings() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(of_note(&mut conn, A, "n").await.unwrap(), None, "not heard yet");
		drop(conn);
		attempt(&pool, "old", 1, "heard", "2026-11-01").await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(of_note(&mut conn, A, "n").await.unwrap(), None, "heard before verdicts were");
		drop(conn);
		attempt(&pool, "m", 1, "heard", "2026-11-14").await;
		let garbled = verdict(Kind::Garbled, &["low_confidence"], Some(signals()));
		judge(&pool, "m", 1, &garbled, "2026-11-14").await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(of_note(&mut conn, A, "n").await.unwrap(), Some(garbled.clone()));
		assert_eq!(of_ending(&mut conn, A, "n", "m").await.unwrap(), Some(garbled));
		assert_eq!(of_ending(&mut conn, A, "n", "old").await.unwrap(), None);
		let appended = sqlx::query("UPDATE voice_transcript_verdict SET verdict = 'heard'").execute(&mut *conn).await;
		assert!(appended.is_err(), "append-only");
	}

	/// HEARD1's essay check judges the transcript the routine reads: an
	/// interview take's verbatim one first, then the highest number filed; a
	/// Rust Rounds take's highest number; nothing before anything is filed.
	#[tokio::test]
	async fn an_essay_judges_the_transcript_the_routine_reads() {
		use super::as_the_routine_reads;
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(as_the_routine_reads(&mut conn, A, "n", true).await.unwrap(), None, "nothing heard");
		drop(conn);
		attempt(&pool, "old", 1, "silent", "2026-11-01").await;
		judge(&pool, "old", 1, &verdict(Kind::NothingSaid, &[], None), "2026-11-01").await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(as_the_routine_reads(&mut conn, A, "n", false).await.unwrap(), None, "not filed: the routine waits");
		drop(conn);
		for (model, n, kind, at) in [("m+verbatim-1", 1, Kind::Garbled, "2026-11-14"), ("plain", 2, Kind::Heard, "2026-11-15")] {
			attempt(&pool, model, 1, "heard", at).await;
			let signals = Some(signals());
			judge(&pool, model, 1, &verdict(kind, &[], signals), at).await;
			filed(&pool, model, n, "added").await;
		}
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(as_the_routine_reads(&mut conn, A, "n", true).await.unwrap(), Some(Kind::Garbled), "the verbatim one first");
		assert_eq!(as_the_routine_reads(&mut conn, A, "n", false).await.unwrap(), Some(Kind::Heard), "else the highest number");
		drop(conn);
		// Filed and not yet taken, or refused, the routine never saw: no
		// winner.
		attempt(&pool, "refused", 1, "heard", "2026-11-16").await;
		judge(&pool, "refused", 1, &verdict(Kind::Heard, &[], Some(signals())), "2026-11-16").await;
		filed(&pool, "refused", 3, "conflict").await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(as_the_routine_reads(&mut conn, A, "n", false).await.unwrap(), Some(Kind::Heard), "n 2 still wins");
		drop(conn);
		attempt(&pool, "untaken", 1, "heard", "2026-11-16").await;
		judge(&pool, "untaken", 1, &verdict(Kind::Garbled, &[], Some(signals())), "2026-11-16").await;
		sqlx::query("INSERT INTO mailbox_transcript (subject_id, note_id, model, n, filed_at) VALUES (?1, 'n', 'untaken', 4, 'now')")
			.bind(A)
			.execute(&pool)
			.await
			.unwrap();
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(as_the_routine_reads(&mut conn, A, "n", false).await.unwrap(), Some(Kind::Heard), "not handed over yet");
		drop(conn);
		attempt(&pool, "unjudged", 1, "heard", "2026-11-16").await;
		filed(&pool, "unjudged", 5, "unchanged").await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(
			as_the_routine_reads(&mut conn, A, "n", false).await.unwrap(),
			None,
			"a winner with no verdict reads as heard"
		);
		assert_eq!(as_the_routine_reads(&mut conn, A, "n", true).await.unwrap(), Some(Kind::Garbled), "verbatim still first");
		assert!(Kind::NothingSaid.may_be_silent() && Kind::Garbled.may_be_silent() && Kind::Failed.may_be_silent());
		assert!(!Kind::Heard.may_be_silent());
	}

	/// A failure is the note's verdict only while nothing ended it.
	#[tokio::test]
	async fn a_final_failure_is_the_verdict_of_a_note_nothing_ended() {
		let pool = pool().await;
		for n in 1..=3 {
			attempt(&pool, "m", n, "deadline", "2026-11-14").await;
		}
		let failed = verdict(Kind::Failed, &["deadline"], None);
		judge(&pool, "m", 3, &failed, "2026-11-14").await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(of_note(&mut conn, A, "n").await.unwrap(), Some(failed));
		drop(conn);
		attempt(&pool, "better", 1, "silent", "2026-11-15").await;
		let nothing = verdict(Kind::NothingSaid, &[], None);
		judge(&pool, "better", 1, &nothing, "2026-11-15").await;
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(of_note(&mut conn, A, "n").await.unwrap(), Some(nothing), "a better model's ending replaces the failure");
	}

	/// paulgsc/server#913: 20261122004001 judges each model's last failed
	/// attempt, numbered at least 3, that ran out before HEARD1, on a take of
	/// every kind (C976): the migration names no kind, and this walks
	/// [`crate::Of::ALL`], which the compiler keeps whole. A kind the schema
	/// before it doesn't know yet is skipped: it can't have taken that ran
	/// out before HEARD1. As `judge` would have; nothing else, nor anything
	/// twice.
	#[tokio::test]
	async fn a_take_that_ran_out_before_verdicts_is_judged_failed() {
		const BEFORE: i64 = 20_261_121_000_500;
		let pool = pool().await;
		MIGRATOR.undo(&pool, BEFORE).await.unwrap();
		let mut known = Vec::new();
		for of in crate::Of::ALL {
			let note = String::from("ran-out-") + of.as_str();
			let held = sqlx::query(
				"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
				 VALUES (?1, ?2, ?3, 'r', 'then', 'audio/webm', 1, 's', 'now')",
			)
			.bind(A)
			.bind(&note)
			.bind(of.as_str())
			.execute(&pool)
			.await;
			match held {
				Ok(_) => known.push(of),
				Err(err) if err.to_string().contains("CHECK constraint failed") => continue,
				Err(err) => panic!("{err}"),
			}
			for n in 1..=3 {
				sqlx::query(
					"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, took_ms, heard_at)
					 VALUES (?1, ?2, 'm', 'e', ?3, 'deadline', 1, '2026-11-10')",
				)
				.bind(A)
				.bind(&note)
				.bind(n)
				.execute(&pool)
				.await
				.unwrap();
			}
		}
		for n in 1..=3 {
			let at = String::from("2026-11-10T0") + &n.to_string() + ":00:00.000Z";
			attempt(&pool, "ran-out", n, if n == 3 { "failed" } else { "deadline" }, &at).await;
			attempt(&pool, "unreadable", n, "unreadable", &at).await;
			attempt(&pool, "too-long", n, "too_long", &at).await;
			attempt(&pool, "deadline", n, "deadline", &at).await;
			attempt(&pool, "judged", n, "too_long", "2026-11-17").await;
		}
		judge(&pool, "judged", 3, &verdict(Kind::Failed, &["too_long"], None), "2026-11-17").await;
		for n in 1..=2 {
			attempt(&pool, "still-trying", n, "unreadable", "2026-11-10").await;
			attempt(&pool, "ended", n, "deadline", "2026-11-10").await;
			attempt(&pool, "spoke", n, "failed", "2026-11-10").await;
		}
		attempt(&pool, "ended", 3, "silent", "2026-11-10").await;
		attempt(&pool, "spoke", 3, "heard", "2026-11-10").await;
		MIGRATOR.run(&pool).await.unwrap();
		let judged: Vec<(String, i64, String, String, String)> =
			sqlx::query_as("SELECT model, attempt, verdict, reasons, judged_at FROM voice_transcript_verdict WHERE note_id = 'n' ORDER BY model, attempt")
				.fetch_all(&pool)
				.await
				.unwrap();
		let at3 = "2026-11-10T03:00:00.000Z";
		let expected = [
			("deadline", 3, "failed", "deadline", at3),
			("judged", 3, "failed", "too_long", "2026-11-17"),
			("ran-out", 3, "failed", "engine", at3),
			("too-long", 3, "failed", "too_long", at3),
			("unreadable", 3, "failed", "unreadable", at3),
		];
		assert_eq!(
			judged,
			expected.map(|(model, attempt, verdict, reasons, at)| (model.to_owned(), attempt, verdict.to_owned(), reasons.to_owned(), at.to_owned())),
			"each judged as judge would, dated at its attempt"
		);
		assert!(known.len() >= 10, "the ten kinds of HEARD1's day are all walked: {known:?}");
		for of in known {
			let kept: Option<(i64, String, String)> = sqlx::query_as("SELECT attempt, verdict, reasons FROM voice_transcript_verdict WHERE note_id = ?1")
				.bind(String::from("ran-out-") + of.as_str())
				.fetch_optional(&pool)
				.await
				.unwrap();
			assert_eq!(kept, Some((3, String::from("failed"), String::from("deadline"))), "{}: every kind is judged", of.as_str());
		}
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(of_ending(&mut conn, A, "n", "ended").await.unwrap(), None, "an ending kept before verdicts stays unjudged");
	}
}
