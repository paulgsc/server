//! The owner's reconciliation of each take home could not hear
//! (paulgsc/server#912, HEARD3, epic paulgsc/some-ui#3070):
//! `voice_note_attention`.
//!
//! `20261116000300_create_voice_note_attention.up.sql` records the design;
//! in short:
//!
//! - **What needs attention.** A note home holds, of any kind, whose newest
//!   verdict ([`crate::verdict::newest`]) is `garbled`, `failed` or
//!   `nothing_said`, and that no act answers yet. [`needing`] lists them,
//!   each with its verdict, codes and signals, so the phone shows each code
//!   for itself and never one "issue" for all of them (the owner's words on
//!   #912).
//! - **An act answers one verdict.** [`act`] appends a row naming the act
//!   ([`Act`]) and the verdict's key. A new verdict, from a model that heard
//!   the note again, is unanswered until the owner acts on it.
//! - **Append-only, and kept for good.** Nothing is updated; a later act is
//!   a new row. The rows outlive the note, as its verdicts do, and leave only
//!   with the account.
//! - **A delete is an act.** `crate::delete::delete` records [`Act::Deleted`]
//!   in its own transaction for a note that needed attention. A note
//!   deleted for good drops out of [`needing`] because the note is gone.
//! - **Hearing a take again is an act** (paulgsc/some-ui#3339):
//!   [`Act::HearAgain`] answers a newest verdict that is `failed` (no
//!   attempt ended the note), and the transcriber gives the note a fresh set
//!   of its attempts, those numbered after the act's
//!   (`crate::transcript::waiting`). It is asked at most once per
//!   [`HEAR_AGAIN_SNOOZE`] on a note, and it keeps the take listed by
//!   [`needing`], with when it was asked ([`HearAgain`]), until a new
//!   verdict comes: what home heard then is answered as any other.
//! - **Per subject.** Every query takes the subject and filters on it.

use crate::{
	transcript::{self, Hearing, Transcript},
	verdict::{self, Kind, Newest, Verdict},
	VoiceError,
};
use serde::Serialize;
use sqlx::SqliteConnection;

/// What the owner did about a take home could not hear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
	/// The owner listened and keeps it as it is.
	Kept,
	/// A new take is to replace it.
	RecordAgain,
	/// The owner deleted it for good, from My data (`crate::delete`).
	Deleted,
	/// The owner asks home to hear it again (paulgsc/some-ui#3339): a fresh
	/// set of the transcriber's attempts, for a take no attempt ended.
	HearAgain,
}

impl Act {
	/// The stored name.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Kept => "kept",
			Self::RecordAgain => "record_again",
			Self::Deleted => "deleted",
			Self::HearAgain => "hear_again",
		}
	}
}

/// How long after the owner asks home to hear a take again before they may
/// ask again.
///
/// A policy, not a deployment's setting (the owner, 2026-11-21,
/// paulgsc/some-ui#3339: a retry "capped, with a snooze"). Six hours lets a
/// fresh set of attempts end, the next night's work included, before
/// another is asked for.
pub const HEAR_AGAIN_SNOOZE: chrono::Duration = chrono::Duration::hours(6);

/// Whether a note's newest verdict asks the owner to act on it.
#[must_use]
pub const fn asks(kind: Kind) -> bool {
	!matches!(kind, Kind::Heard)
}

/// A take that needs attention, as the phone's reconciliation page reads it.
/// Serialized in `camelCase`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Take {
	/// The note's id.
	pub id: String,
	/// What it was said for.
	pub of: String,
	/// Which one.
	#[serde(rename = "ref")]
	pub of_ref: String,
	/// When the phone recorded it.
	pub recorded_at: String,
	/// What home heard in it, as `GET /voice-notes/:id/heard` gives it; none
	/// for a note no attempt ended.
	pub heard: Option<Transcript>,
	/// Its newest verdict, which no act answers yet but a request to hear
	/// it again.
	pub verdict: Verdict,
	/// Whether the owner may ask home to hear it again, and when they last
	/// did.
	pub hear_again: HearAgain,
}

/// Where a take stands with hearing it again (paulgsc/some-ui#3339).
/// Serialized in `camelCase`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HearAgain {
	/// Whether `POST /voice-notes/:id/attention` with `again` would be
	/// taken now: its verdict is `failed`, home is not hearing it again
	/// already, and the snooze has passed.
	pub open: bool,
	/// Home is hearing it again: the newest verdict is answered by a
	/// request, and no new verdict has come.
	pub hearing: bool,
	/// When the owner last asked, if ever.
	pub asked_at: Option<String>,
	/// When they may next ask, while the snooze lasts.
	pub next_at: Option<String>,
}

/// The newest request to hear `subject`'s note `note_id` again: when, and
/// the verdict it answered.
async fn newest_request(conn: &mut SqliteConnection, subject: &str, note_id: &str) -> Result<Option<(String, String, i64, String)>, VoiceError> {
	let row = sqlx::query!(
		r#"SELECT acted_at AS "acted_at!", model AS "model!", attempt AS "attempt!: i64", rule AS "rule!" FROM voice_note_attention
		   WHERE subject_id = ?1 AND note_id = ?2 AND act = 'hear_again'
		   ORDER BY n DESC LIMIT 1"#,
		subject,
		note_id
	)
	.fetch_optional(&mut *conn)
	.await?;
	Ok(row.map(|row| (row.acted_at, row.model, row.attempt, row.rule)))
}

/// When the snooze after a request asked at `asked_at` ends, if it is
/// still running at `now`. A stamp that does not read is no snooze.
fn snoozed_until(asked_at: &str, now: &str) -> Option<String> {
	let asked = chrono::DateTime::parse_from_rfc3339(asked_at).ok()?;
	let now = chrono::DateTime::parse_from_rfc3339(now).ok()?;
	let until = asked + HEAR_AGAIN_SNOOZE;
	(now < until).then(|| until.with_timezone(&chrono::Utc).to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

/// Where `subject`'s note `note_id`, its newest verdict `newest`, stands
/// with hearing it again at `now`.
async fn hear_again_of(conn: &mut SqliteConnection, subject: &str, note_id: &str, newest: (&str, i64, &Verdict), now: &str) -> Result<HearAgain, VoiceError> {
	let (model, attempt, verdict) = newest;
	let request = newest_request(conn, subject, note_id).await?;
	let hearing = request
		.as_ref()
		.is_some_and(|(_, of_model, of_attempt, of_rule)| of_model == model && *of_attempt == attempt && *of_rule == verdict.rule);
	let next_at = request.as_ref().and_then(|(asked_at, ..)| snoozed_until(asked_at, now));
	Ok(HearAgain {
		open: verdict.kind == Kind::Failed && !hearing && next_at.is_none(),
		hearing,
		asked_at: request.map(|(asked_at, ..)| asked_at),
		next_at,
	})
}

/// Whether home is hearing `subject`'s note `note_id` again.
///
/// Its newest verdict's newest act is a request to hear it again
/// (paulgsc/some-ui#3339). A reader waiting on its words (`take_files`)
/// waits for them, as for a take not heard yet, rather than taking the old
/// verdict as final.
///
/// # Errors
/// Any `sqlx` failure, or a stored verdict this build does not know.
pub async fn hearing_again(conn: &mut SqliteConnection, subject: &str, note_id: &str) -> Result<bool, VoiceError> {
	let Some(newest) = verdict::newest(conn, subject, note_id).await? else {
		return Ok(false);
	};
	Ok(answer_of(conn, subject, note_id, &newest).await?.as_deref() == Some(Act::HearAgain.as_str()))
}

/// The most takes [`needing`] answers: the newest recorded.
pub const LISTED: usize = 100;

/// The newest act on `subject`'s note `note_id` that answers `newest`.
async fn answer_of(conn: &mut SqliteConnection, subject: &str, note_id: &str, newest: &Newest) -> Result<Option<String>, VoiceError> {
	Ok(
		sqlx::query_scalar!(
			"SELECT act FROM voice_note_attention
		 WHERE subject_id = ?1 AND note_id = ?2 AND model = ?3 AND attempt = ?4 AND rule = ?5
		 ORDER BY n DESC LIMIT 1",
			subject,
			note_id,
			newest.model,
			newest.attempt,
			newest.verdict.rule
		)
		.fetch_optional(&mut *conn)
		.await?,
	)
}

/// `subject`'s takes that need attention, newest recorded first (the note's
/// id breaking a tie), at most [`LISTED`] of them, and how many in all.
///
/// One statement: for each note held with a verdict not heard that no act
/// answers (the verdicts' primary key leads with the note; every note listed
/// is one, and a note reconciled since is not), its newest verdict, chosen as
/// [`verdict::newest`] chooses it, kept when not heard and no act answers it,
/// then counted and cut to [`LISTED`]. Only the listed takes' transcripts are
/// read after. Run it in one transaction, so the list and the total agree.
///
/// # Errors
/// Any `sqlx` failure, or a stored verdict this build does not know.
pub async fn needing(conn: &mut SqliteConnection, subject: &str, now: &str) -> Result<(Vec<Take>, i64), VoiceError> {
	let limit = i64::try_from(LISTED).unwrap_or(i64::MAX);
	// The chosen verdict is `verdict::newest`'s: the verdict of the ending
	// `transcript::heard` chooses, else, with no ending, the newest failure;
	// the newest judged, then the newest rule. Keep the two in step.
	let rows = sqlx::query!(
		r#"WITH chosen AS (
		     SELECT n.note_id, n.of_kind, n.of_ref, n.recorded_at,
		            (SELECT v.rowid FROM voice_transcript_verdict v
		             WHERE v.subject_id = n.subject_id AND v.note_id = n.note_id
		               AND ((v.model, v.attempt) = (
		                      SELECT t.model, t.attempt FROM voice_transcript t
		                      WHERE t.subject_id = n.subject_id AND t.note_id = n.note_id AND t.outcome IN ('heard', 'silent')
		                      ORDER BY t.outcome = 'heard' DESC, t.heard_at DESC, t.model DESC
		                      LIMIT 1)
		                    OR (NOT EXISTS (
		                          SELECT 1 FROM voice_transcript t
		                          WHERE t.subject_id = n.subject_id AND t.note_id = n.note_id AND t.outcome IN ('heard', 'silent'))
		                        AND v.verdict = 'failed'))
		             ORDER BY v.judged_at DESC, v.rule DESC, v.model DESC, v.attempt DESC
		             LIMIT 1) AS chosen_row
		     FROM voice_note n
		     WHERE n.subject_id = ?1
		       AND EXISTS (
		         SELECT 1 FROM voice_transcript_verdict w
		         WHERE w.subject_id = n.subject_id AND w.note_id = n.note_id AND w.verdict <> 'heard'
		           AND NOT EXISTS (
		             SELECT 1 FROM voice_note_attention a
		             WHERE a.subject_id = w.subject_id AND a.note_id = w.note_id
		               AND a.model = w.model AND a.attempt = w.attempt AND a.rule = w.rule
		               AND a.act <> 'hear_again')))
		   SELECT c.note_id AS "note_id!", c.of_kind AS "of_kind!", c.of_ref AS "of_ref!", c.recorded_at AS "recorded_at!",
		          v.rule AS "rule!", v.verdict AS "verdict!", v.reasons AS "reasons!", v.avg_logprob, v.compression_ratio,
		          v.no_speech_prob, v.unsure_share, v.language, v.language_p, v.expected_language, v.words_per_second,
		          v.speech_ms, v.peak_dbfs, v.rms_dbfs, v.model AS "model!", v.attempt AS "attempt!: i64",
		          COUNT(*) OVER () AS "total!: i64"
		   FROM chosen c
		   JOIN voice_transcript_verdict v ON v.rowid = c.chosen_row
		   WHERE v.verdict <> 'heard'
		     AND NOT EXISTS (
		       SELECT 1 FROM voice_note_attention a
		       WHERE a.subject_id = ?1 AND a.note_id = c.note_id
		         AND a.model = v.model AND a.attempt = v.attempt AND a.rule = v.rule
		         AND a.act <> 'hear_again')
		   ORDER BY c.recorded_at DESC, c.note_id DESC
		   LIMIT ?2"#,
		subject,
		limit
	)
	.fetch_all(&mut *conn)
	.await?;
	let total = rows.first().map_or(0, |row| row.total);
	let mut takes = Vec::with_capacity(rows.len());
	for row in rows {
		let verdict = verdict::read(verdict::Row {
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
		let heard = match transcript::heard(conn, subject, &row.note_id).await? {
			Hearing::Heard(transcript) => Some(transcript),
			Hearing::Waiting | Hearing::NotHeld => None,
		};
		let hear_again = hear_again_of(conn, subject, &row.note_id, (&row.model, row.attempt, &verdict), now).await?;
		takes.push(Take {
			id: row.note_id,
			of: row.of_kind,
			of_ref: row.of_ref,
			recorded_at: row.recorded_at,
			heard,
			verdict,
			hear_again,
		});
	}
	Ok((takes, total))
}

/// What [`act`] did.
#[derive(Debug, Clone, PartialEq)]
pub enum Acted {
	/// The act is recorded, answering this verdict.
	Recorded(Verdict),
	/// The newest act on this verdict is already this one: nothing written.
	Unchanged(Verdict),
	/// The note's newest verdict asks for nothing (heard, or none yet):
	/// nothing written.
	NothingAsked,
	/// The subject holds no such note: nothing written.
	NotHeld,
	/// [`Act::HearAgain`] on a take an attempt ended (`garbled` or
	/// `nothing_said`): the same model would hear the same, so nothing is
	/// written.
	Ended(Verdict),
	/// [`Act::HearAgain`] asked within [`HEAR_AGAIN_SNOOZE`] of the last
	/// request: nothing written; it may be asked from this time.
	Snoozed { verdict: Verdict, next_at: String },
}

/// Records `subject`'s act `act` on their note `note_id` at `now`, answering
/// its newest verdict.
///
/// A verdict already answered by another act is answered again (a later act
/// is a new row); one whose newest act is this one writes nothing, so a
/// retry is answered as it was. Run it in a `BEGIN IMMEDIATE` transaction,
/// so the read and the write see one snapshot.
///
/// # Errors
/// Any `sqlx` failure, or a stored verdict this build does not know.
pub async fn act(conn: &mut SqliteConnection, subject: &str, note_id: &str, act: Act, now: &str) -> Result<Acted, VoiceError> {
	let held = sqlx::query_scalar!(
		r#"SELECT EXISTS (SELECT 1 FROM voice_note WHERE subject_id = ?1 AND note_id = ?2) AS "held!: bool""#,
		subject,
		note_id
	)
	.fetch_one(&mut *conn)
	.await?;
	if !held {
		return Ok(Acted::NotHeld);
	}
	let Some(newest) = verdict::newest(conn, subject, note_id).await? else {
		return Ok(Acted::NothingAsked);
	};
	if !asks(newest.verdict.kind) {
		return Ok(Acted::NothingAsked);
	}
	if answer_of(conn, subject, note_id, &newest).await?.as_deref() == Some(act.as_str()) {
		return Ok(Acted::Unchanged(newest.verdict));
	}
	if act == Act::HearAgain {
		if newest.verdict.kind != Kind::Failed {
			return Ok(Acted::Ended(newest.verdict));
		}
		if let Some(next_at) = newest_request(conn, subject, note_id).await?.and_then(|(asked_at, ..)| snoozed_until(&asked_at, now)) {
			return Ok(Acted::Snoozed { verdict: newest.verdict, next_at });
		}
	}
	let name = act.as_str();
	sqlx::query!(
		"INSERT INTO voice_note_attention (subject_id, note_id, n, act, model, attempt, rule, acted_at)
		 VALUES (?1, ?2, (SELECT COALESCE(MAX(n), 0) + 1 FROM voice_note_attention WHERE subject_id = ?1 AND note_id = ?2), ?3, ?4, ?5, ?6, ?7)",
		subject,
		note_id,
		name,
		newest.model,
		newest.attempt,
		newest.verdict.rule,
		now
	)
	.execute(&mut *conn)
	.await?;
	Ok(Acted::Recorded(newest.verdict))
}

#[cfg(test)]
mod tests {
	use super::{act, needing, Act, Acted};
	use crate::verdict::{keep, Judged, Kind, Verdict};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";
	const B: &str = "subject-b";

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

	async fn note(pool: &SqlitePool, subject: &str, id: &str, of: &str, at: &str) {
		sqlx::query(
			"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
			 VALUES (?1, ?2, ?3, 'r', ?4, 'audio/webm', 1, 's', 'now')",
		)
		.bind(subject)
		.bind(id)
		.bind(of)
		.bind(at)
		.execute(pool)
		.await
		.unwrap();
	}

	/// An ending of `model` on the note, judged `kind`.
	async fn heard(pool: &SqlitePool, subject: &str, id: &str, model: &str, kind: Kind, at: &str) {
		let outcome = if kind == Kind::NothingSaid { "silent" } else { "heard" };
		let spoke = outcome == "heard";
		sqlx::query(
			"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, language, language_p, duration_ms, text, took_ms, heard_at)
			 VALUES (?1, ?2, ?3, 'e', 1, ?4, ?5, ?6, 1000, ?7, 1, ?8)",
		)
		.bind(subject)
		.bind(id)
		.bind(model)
		.bind(outcome)
		.bind(spoke.then_some("en"))
		.bind(spoke.then_some(0.9))
		.bind(spoke.then_some("words"))
		.bind(at)
		.execute(pool)
		.await
		.unwrap();
		let mut conn = pool.acquire().await.unwrap();
		let verdict = Verdict {
			kind,
			reasons: if kind == Kind::Garbled { vec![String::from("low_confidence")] } else { Vec::new() },
			rule: String::from("heard-1"),
			signals: None,
		};
		let judged = Judged {
			subject,
			note_id: id,
			model,
			attempt: 1,
		};
		// A test verdict carries no signals, so a heard one is written by hand.
		if spoke {
			sqlx::query(
				"INSERT INTO voice_transcript_verdict (subject_id, note_id, model, attempt, rule, verdict, reasons, speech_ms, judged_at)
				 VALUES (?1, ?2, ?3, 1, 'heard-1', ?4, ?5, 1000, ?6)",
			)
			.bind(subject)
			.bind(id)
			.bind(model)
			.bind(kind.as_str())
			.bind(verdict.reasons.join(" "))
			.bind(at)
			.execute(&mut *conn)
			.await
			.unwrap();
		} else {
			keep(&mut conn, judged, &verdict, at).await.unwrap();
		}
	}

	async fn listed(pool: &SqlitePool, subject: &str) -> (Vec<String>, i64) {
		let mut conn = pool.acquire().await.unwrap();
		let (takes, total) = needing(&mut conn, subject, "2026-11-21T00:00:00.000Z").await.unwrap();
		(takes.into_iter().map(|take| take.id).collect(), total)
	}

	async fn acted(pool: &SqlitePool, subject: &str, id: &str, what: Act) -> Acted {
		let mut conn = pool.acquire().await.unwrap();
		act(&mut conn, subject, id, what, "2026-11-16T20:00:00.000Z").await.unwrap()
	}

	/// Every kind of note whose newest verdict is not heard is listed, newest
	/// recorded first, with its codes; a heard one and another subject's are
	/// not.
	#[tokio::test]
	async fn every_unheard_take_of_any_kind_needs_attention() {
		let pool = pool().await;
		note(&pool, A, "garbled", "studio", "2026-11-01").await;
		note(&pool, A, "silent", "reflection", "2026-11-02").await;
		note(&pool, A, "fine", "rust-round", "2026-11-03").await;
		note(&pool, A, "waiting", "studio", "2026-11-04").await;
		note(&pool, B, "theirs", "studio", "2026-11-05").await;
		heard(&pool, A, "garbled", "m", Kind::Garbled, "2026-11-10").await;
		heard(&pool, A, "silent", "m", Kind::NothingSaid, "2026-11-10").await;
		heard(&pool, A, "fine", "m", Kind::Heard, "2026-11-10").await;
		heard(&pool, B, "theirs", "m", Kind::Garbled, "2026-11-10").await;

		let mut conn = pool.acquire().await.unwrap();
		let (takes, total) = needing(&mut conn, A, "2026-11-21T00:00:00.000Z").await.unwrap();
		drop(conn);
		assert_eq!(total, 2);
		assert_eq!(takes.iter().map(|take| take.id.as_str()).collect::<Vec<_>>(), ["silent", "garbled"]);
		let garbled = takes.get(1).unwrap();
		assert_eq!(garbled.verdict.kind, Kind::Garbled);
		assert_eq!(garbled.verdict.reasons, ["low_confidence"]);
		assert_eq!(garbled.heard.as_ref().unwrap().text, "words");
		let json = serde_json::to_value(garbled).unwrap();
		assert_eq!(json["ref"], "r");
		assert_eq!(json["recordedAt"], "2026-11-01");
		assert_eq!(listed(&pool, B).await, (vec![String::from("theirs")], 1));
	}

	/// An act answers the newest verdict and takes the note off the list; a
	/// retry writes nothing; another act is a new row; a new verdict asks
	/// again; nothing is updated.
	#[tokio::test]
	async fn an_act_answers_the_newest_verdict_and_is_kept() {
		let pool = pool().await;
		note(&pool, A, "t", "studio", "2026-11-01").await;
		assert_eq!(acted(&pool, B, "t", Act::Kept).await, Acted::NotHeld, "another subject's");
		assert_eq!(acted(&pool, A, "t", Act::Kept).await, Acted::NothingAsked, "not heard yet");
		heard(&pool, A, "t", "m", Kind::Garbled, "2026-11-10").await;
		assert!(matches!(acted(&pool, A, "t", Act::Kept).await, Acted::Recorded(v) if v.kind == Kind::Garbled));
		assert_eq!(listed(&pool, A).await, (Vec::new(), 0), "answered");
		assert!(matches!(acted(&pool, A, "t", Act::Kept).await, Acted::Unchanged(_)), "a retry");
		assert!(matches!(acted(&pool, A, "t", Act::RecordAgain).await, Acted::Recorded(_)), "a later act");
		let rows: Vec<(i64, String)> = sqlx::query_as("SELECT n, act FROM voice_note_attention ORDER BY n").fetch_all(&pool).await.unwrap();
		assert_eq!(rows, [(1, String::from("kept")), (2, String::from("record_again"))]);
		assert!(sqlx::query("UPDATE voice_note_attention SET act = 'kept'").execute(&pool).await.is_err(), "append-only");

		// A better model hears it again: a new verdict, unanswered.
		heard(&pool, A, "t", "better", Kind::Garbled, "2026-11-12").await;
		assert_eq!(listed(&pool, A).await, (vec![String::from("t")], 1));
		heard(&pool, A, "t", "best", Kind::Heard, "2026-11-13").await;
		assert_eq!(listed(&pool, A).await, (Vec::new(), 0), "heard now");
		assert_eq!(acted(&pool, A, "t", Act::Kept).await, Acted::NothingAsked);

		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
		let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voice_note_attention").fetch_one(&pool).await.unwrap();
		assert_eq!(left, 0, "leaves with the account");
	}

	/// The list chooses each note's verdict as `verdict::newest` does: a
	/// final failure while nothing ended the note, then the ending's; an old
	/// verdict no act answers never lists a note whose newest is heard.
	#[tokio::test]
	async fn the_list_reads_the_verdict_newest_chooses() {
		let pool = pool().await;
		note(&pool, A, "t", "interview", "2026-11-01").await;
		sqlx::query(
			"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, took_ms, heard_at)
			 VALUES (?1, 't', 'm', 'e', 1, 'deadline', 1, '2026-11-09')",
		)
		.bind(A)
		.execute(&pool)
		.await
		.unwrap();
		let failed = Verdict {
			kind: Kind::Failed,
			reasons: vec![String::from("deadline")],
			rule: String::from("heard-1"),
			signals: None,
		};
		let judged = Judged {
			subject: A,
			note_id: "t",
			model: "m",
			attempt: 1,
		};
		let mut conn = pool.acquire().await.unwrap();
		keep(&mut conn, judged, &failed, "2026-11-09").await.unwrap();
		let (takes, _) = needing(&mut conn, A, "2026-11-21T00:00:00.000Z").await.unwrap();
		assert_eq!(takes.first().map(|take| (take.verdict.kind, take.heard.is_none())), Some((Kind::Failed, true)));
		drop(conn);
		heard(&pool, A, "t", "better", Kind::NothingSaid, "2026-11-10").await;
		let mut conn = pool.acquire().await.unwrap();
		let (takes, _) = needing(&mut conn, A, "2026-11-21T00:00:00.000Z").await.unwrap();
		let newest = crate::verdict::newest(&mut conn, A, "t").await.unwrap().unwrap();
		assert_eq!(takes.first().map(|take| take.verdict.clone()), Some(newest.verdict), "the ending replaces the failure");
		drop(conn);
		heard(&pool, A, "t", "best", Kind::Heard, "2026-11-11").await;
		assert_eq!(listed(&pool, A).await, (Vec::new(), 0), "heard now, though the failure is unanswered");
	}

	/// Only the newest [`super::LISTED`] are listed; the total counts all.
	#[tokio::test]
	async fn the_list_is_the_newest_and_the_total_counts_all() {
		let pool = pool().await;
		let many = super::LISTED + 2;
		// `n` as its last `width` digits, zero-padded.
		let digits = |n: usize, width: u32| -> String { (n + 10_usize.pow(width)).to_string().chars().skip(1).collect() };
		let id_of = |n: usize| String::from("t") + &digits(n, 3);
		for n in 0..many {
			let id = id_of(n);
			let at = String::from("2026-11-01T00:") + &digits(n / 60, 2) + ":" + &digits(n % 60, 2);
			note(&pool, A, &id, "studio", &at).await;
			heard(&pool, A, &id, "m", Kind::Garbled, "2026-11-10").await;
		}
		let (ids, total) = listed(&pool, A).await;
		assert_eq!(total, i64::try_from(many).unwrap());
		assert_eq!(ids.len(), super::LISTED);
		assert_eq!(ids.first().unwrap(), &id_of(many - 1));
	}

	/// `model`'s failed attempt `n` on `subject`'s note `id`, and, when it is
	/// final, its `failed` verdict.
	async fn failed(pool: &SqlitePool, id: &str, n: i64, last: bool, at: &str) {
		sqlx::query(
			"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, took_ms, heard_at)
			 VALUES (?1, ?2, 'm', 'e', ?3, 'deadline', 1, ?4)",
		)
		.bind(A)
		.bind(id)
		.bind(n)
		.bind(at)
		.execute(pool)
		.await
		.unwrap();
		if last {
			let verdict = Verdict {
				kind: Kind::Failed,
				reasons: vec![String::from("deadline")],
				rule: String::from("heard-1"),
				signals: None,
			};
			let judged = Judged {
				subject: A,
				note_id: id,
				model: "m",
				attempt: n,
			};
			let mut conn = pool.acquire().await.unwrap();
			keep(&mut conn, judged, &verdict, at).await.unwrap();
		}
	}

	async fn hear_again_at(pool: &SqlitePool, id: &str, now: &str) -> Acted {
		let mut conn = pool.acquire().await.unwrap();
		act(&mut conn, A, id, Act::HearAgain, now).await.unwrap()
	}

	async fn standing(pool: &SqlitePool, id: &str, now: &str) -> super::HearAgain {
		let mut conn = pool.acquire().await.unwrap();
		let (takes, _) = needing(&mut conn, A, now).await.unwrap();
		takes.into_iter().find(|take| take.id == id).map(|take| take.hear_again).unwrap()
	}

	/// What waits on model `m`, and each note's attempts spent before its
	/// newest request.
	async fn waiting_spent(pool: &SqlitePool) -> Vec<(String, i64)> {
		let mut conn = pool.acquire().await.unwrap();
		let keys = crate::transcript::Keys {
			plain: "m",
			verbatim: "m+verbatim-1",
		};
		crate::transcript::waiting(&mut conn, keys, 3, 20)
			.await
			.unwrap()
			.into_iter()
			.map(|waiting| (waiting.note_id, waiting.spent))
			.collect()
	}

	/// paulgsc/some-ui#3339: the owner asks home to hear a failed take again.
	/// The take gets a fresh set of attempts, stays listed while home hears
	/// it, and is asked again only once the snooze has passed; a take an
	/// attempt ended is never heard again; no row is rewritten.
	#[tokio::test]
	async fn a_failed_take_heard_again_gets_a_fresh_set_of_attempts_once_per_snooze() {
		let pool = pool().await;
		note(&pool, A, "t", "studio", "2026-11-01").await;
		sqlx::query("INSERT INTO voice_note_audio (subject_id, note_id, body) VALUES (?1, 't', x'00')")
			.bind(A)
			.execute(&pool)
			.await
			.unwrap();
		assert_eq!(waiting_spent(&pool).await, [(String::from("t"), 0)], "never heard");
		for n in 1..=3 {
			failed(&pool, "t", n, n == 3, "2026-11-20T01:00:00.000Z").await;
		}
		assert!(waiting_spent(&pool).await.is_empty(), "its three attempts spent");
		let open = standing(&pool, "t", "2026-11-21T00:00:00.000Z").await;
		assert!(open.open && !open.hearing && open.asked_at.is_none() && open.next_at.is_none(), "{open:?}");

		assert!(matches!(hear_again_at(&pool, "t", "2026-11-21T00:00:00.000Z").await, Acted::Recorded(v) if v.kind == Kind::Failed));
		assert!(matches!(hear_again_at(&pool, "t", "2026-11-21T00:30:00.000Z").await, Acted::Unchanged(_)), "a resend");
		assert_eq!(waiting_spent(&pool).await, [(String::from("t"), 3)], "a fresh set, after attempt 3");
		let hearing = standing(&pool, "t", "2026-11-21T01:00:00.000Z").await;
		assert_eq!(
			hearing,
			super::HearAgain {
				open: false,
				hearing: true,
				asked_at: Some(String::from("2026-11-21T00:00:00.000Z")),
				next_at: Some(String::from("2026-11-21T06:00:00.000Z")),
			},
			"still listed while home hears it again"
		);
		let mut conn = pool.acquire().await.unwrap();
		assert!(super::hearing_again(&mut conn, A, "t").await.unwrap());
		drop(conn);

		// The fresh set fails too: a new verdict, which no act answers.
		failed(&pool, "t", 4, false, "2026-11-21T02:00:00.000Z").await;
		assert_eq!(waiting_spent(&pool).await, [(String::from("t"), 3)], "one of its fresh three used");
		failed(&pool, "t", 5, false, "2026-11-21T02:10:00.000Z").await;
		failed(&pool, "t", 6, true, "2026-11-21T02:20:00.000Z").await;
		assert!(waiting_spent(&pool).await.is_empty(), "the fresh set spent");
		let mut conn = pool.acquire().await.unwrap();
		assert!(!super::hearing_again(&mut conn, A, "t").await.unwrap(), "a new verdict came");
		drop(conn);
		let snoozed = standing(&pool, "t", "2026-11-21T03:00:00.000Z").await;
		assert!(
			!snoozed.open && !snoozed.hearing && snoozed.next_at.as_deref() == Some("2026-11-21T06:00:00.000Z"),
			"{snoozed:?}"
		);
		assert_eq!(
			hear_again_at(&pool, "t", "2026-11-21T03:00:00.000Z").await,
			Acted::Snoozed {
				verdict: Verdict {
					kind: Kind::Failed,
					reasons: vec![String::from("deadline")],
					rule: String::from("heard-1"),
					signals: None,
				},
				next_at: String::from("2026-11-21T06:00:00.000Z"),
			}
		);
		assert!(standing(&pool, "t", "2026-11-21T06:00:00.000Z").await.open, "the snooze has passed");
		assert!(matches!(hear_again_at(&pool, "t", "2026-11-21T06:00:00.000Z").await, Acted::Recorded(_)));
		assert_eq!(waiting_spent(&pool).await, [(String::from("t"), 6)], "another fresh set, after attempt 6");

		// A take an attempt ended (garbled) is never heard again.
		note(&pool, A, "g", "studio", "2026-11-02").await;
		heard(&pool, A, "g", "m", Kind::Garbled, "2026-11-10").await;
		assert!(!standing(&pool, "g", "2026-11-21T00:00:00.000Z").await.open);
		assert!(matches!(hear_again_at(&pool, "g", "2026-11-21T00:00:00.000Z").await, Acted::Ended(_)));

		let acts: Vec<(String, i64, i64)> = sqlx::query_as("SELECT act, attempt, n FROM voice_note_attention WHERE note_id = 't' ORDER BY n")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(acts, [(String::from("hear_again"), 3, 1), (String::from("hear_again"), 6, 2)], "one row per request");
		let attempts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voice_transcript WHERE note_id = 't'")
			.fetch_one(&pool)
			.await
			.unwrap();
		assert_eq!(attempts, 6, "no attempt rewritten");
	}
}
