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
//!   [`Act::HearAgain`] answers a note's newest verdict, whatever it is, a
//!   heard one included, and the transcriber hears the note afresh under a
//!   new key with a fresh set of its attempts (`crate::transcript::again`,
//!   `crate::transcript::waiting`). It is asked at most once per
//!   [`HEAR_AGAIN_SNOOZE`] on a note. It answers its verdict as any act
//!   does, and the attempts it supersedes (those of its model numbered up
//!   to the one it answered) are skipped by every reader that chooses an
//!   ending or a verdict, so while home hears the take again it reads as
//!   not heard yet everywhere, and when the fresh set ends its attempt is
//!   what is read: nothing is rewritten, and no clock is read. Whether
//!   home is hearing it ([`HearAgain::hearing`]) is told by what is on home,
//!   not by the acts.
//! - **Per subject.** Every query takes the subject and filters on it.

use crate::{
	transcript::{self, Hearing, Keys, Transcript},
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
	/// transcription, under a new key, of any take home has judged.
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
	/// Whether `POST /voice-notes/:id/attention` with `hear-again` would be
	/// taken now: home has judged it, would hear it, is not hearing it again
	/// already, and the snooze has passed. `nextAt` is null whenever this is
	/// true.
	pub open: bool,
	/// Home is hearing it again: the newest verdict is answered by a
	/// request, and no new verdict has come.
	pub hearing: bool,
	/// When the owner last asked, if ever.
	pub asked_at: Option<String>,
	/// When they may next ask, while the snooze lasts.
	pub next_at: Option<String>,
}

/// A request to hear a note again: when, the verdict it answered by that
/// verdict's key, and the key the note is heard again under.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Request {
	asked_at: String,
	model: String,
	attempt: i64,
	rule: String,
	hears_under: String,
}

/// The newest request to hear `subject`'s note `note_id` again.
async fn newest_request(conn: &mut SqliteConnection, subject: &str, note_id: &str) -> Result<Option<Request>, VoiceError> {
	Ok(
		sqlx::query_as!(
			Request,
			r#"SELECT acted_at AS "asked_at!", model AS "model!", attempt AS "attempt!: i64", rule AS "rule!", hears_under AS "hears_under!"
			   FROM voice_note_attention
			   WHERE subject_id = ?1 AND note_id = ?2 AND act = 'hear_again'
			   ORDER BY n DESC LIMIT 1"#,
			subject,
			note_id
		)
		.fetch_optional(&mut *conn)
		.await?,
	)
}

/// The note's kind, and whether home would hear it at all: its recording
/// is held, and the owner has not set it aside (a Jobs note is heard though
/// set aside), as `crate::transcript::waiting` and `voice_note_superseding`
/// ask. `None` for a note the subject does not hold.
async fn hearable(conn: &mut SqliteConnection, subject: &str, note_id: &str) -> Result<Option<(String, bool)>, VoiceError> {
	let row = sqlx::query!(
		r#"SELECT n.of_kind AS "of_kind!",
		          EXISTS (SELECT 1 FROM voice_note_audio a WHERE a.subject_id = n.subject_id AND a.note_id = n.note_id)
		          AND (n.of_kind = 'jobs' OR NOT EXISTS (SELECT 1 FROM voice_note_set_aside s
		                                                 WHERE s.subject_id = n.subject_id AND s.note_id = n.note_id)) AS "hearable!: bool"
		   FROM voice_note n WHERE n.subject_id = ?1 AND n.note_id = ?2"#,
		subject,
		note_id
	)
	.fetch_optional(&mut *conn)
	.await?;
	Ok(row.map(|row| (row.of_kind, row.hearable)))
}

/// The key `keys` hears a note said for `of_kind` under, before any request:
/// the verbatim key for an interview take, the plain one for every other.
fn base_for<'a>(of_kind: &str, keys: Keys<'a>) -> &'a str {
	if of_kind == transcript::VERBATIM {
		keys.verbatim
	} else {
		keys.plain
	}
}

/// The request home is hearing `subject`'s note `note_id` again for, if
/// any: told by what is on home, never by the acts alone. Its newest request
/// stands while its key, `hears_under`, has not ended the note or been
/// judged final, home would still hear the note ([`hearable`]), and its key
/// is one of the model `keys` hears with: after a change of model the
/// request is moot, and the owner may ask again under the new one. So a
/// later act, a recording let go, a mark set aside or a new model never
/// leaves a take "hearing" for good.
async fn hearing(conn: &mut SqliteConnection, subject: &str, note_id: &str, keys: Keys<'_>) -> Result<Option<Request>, VoiceError> {
	let Some(request) = newest_request(conn, subject, note_id).await? else {
		return Ok(None);
	};
	let Some((of_kind, true)) = hearable(conn, subject, note_id).await? else {
		return Ok(None);
	};
	if transcript::base_of(&request.hears_under) != base_for(&of_kind, keys) {
		return Ok(None);
	}
	let done = sqlx::query_scalar!(
		r#"SELECT EXISTS (SELECT 1 FROM voice_transcript t
		                  WHERE t.subject_id = ?1 AND t.note_id = ?2 AND t.model = ?3 AND t.outcome IN ('heard', 'silent'))
		       OR EXISTS (SELECT 1 FROM voice_transcript_verdict v
		                  WHERE v.subject_id = ?1 AND v.note_id = ?2 AND v.model = ?3) AS "done!: bool""#,
		subject,
		note_id,
		request.hears_under
	)
	.fetch_one(&mut *conn)
	.await?;
	Ok((!done).then_some(request))
}

/// The kind of the verdict the owner's newest request to hear `subject`'s
/// note `note_id` again answered, if any.
///
/// While home hears a take again the verdict it answered is skipped, so the
/// rule that lets the owner delete a take home could not hear
/// (`crate::delete::allows`) reads this in its place, there and in My data's
/// `deletable` (`crate::kept`, the same subquery): asking to hear a take again
/// never makes it undeletable.
///
/// # Errors
/// Any `sqlx` failure, or a stored verdict this build does not know.
pub async fn asked_of(conn: &mut SqliteConnection, subject: &str, note_id: &str) -> Result<Option<Kind>, VoiceError> {
	let Some(request) = newest_request(conn, subject, note_id).await? else {
		return Ok(None);
	};
	Ok(answered(conn, subject, note_id, &request).await?.map(|verdict| verdict.kind))
}

/// The verdict a request answered, read by its key.
async fn answered(conn: &mut SqliteConnection, subject: &str, note_id: &str, request: &Request) -> Result<Option<Verdict>, VoiceError> {
	let row = sqlx::query_as!(
		verdict::Row,
		r#"SELECT rule AS "rule!", verdict AS "verdict!", reasons AS "reasons!", avg_logprob, compression_ratio, no_speech_prob,
		          unsure_share, language, language_p, expected_language, words_per_second, speech_ms, peak_dbfs, rms_dbfs
		   FROM voice_transcript_verdict
		   WHERE subject_id = ?1 AND note_id = ?2 AND model = ?3 AND attempt = ?4 AND rule = ?5"#,
		subject,
		note_id,
		request.model,
		request.attempt,
		request.rule
	)
	.fetch_optional(&mut *conn)
	.await?;
	row.map(verdict::read).transpose()
}

/// The verdict an act on `subject`'s note `note_id` answers: its newest
/// ([`verdict::newest`]), or, while a request supersedes every key but one
/// that has not judged the note yet, the verdict that request answered. So
/// an act taken while home hears a take again (a delete's included) still
/// answers, and names, what the owner was looking at.
async fn answerable(conn: &mut SqliteConnection, subject: &str, note_id: &str) -> Result<Option<Newest>, VoiceError> {
	if let Some(newest) = verdict::newest(conn, subject, note_id).await? {
		return Ok(Some(newest));
	}
	let Some(request) = newest_request(conn, subject, note_id).await? else {
		return Ok(None);
	};
	Ok(answered(conn, subject, note_id, &request).await?.map(|verdict| Newest {
		model: request.model,
		attempt: request.attempt,
		verdict,
	}))
}

/// When the snooze after a request asked at `asked_at` ends, if it is
/// still running at `now`. A stamp that does not read is no snooze.
fn snoozed_until(asked_at: &str, now: &str) -> Option<String> {
	let asked = chrono::DateTime::parse_from_rfc3339(asked_at).ok()?;
	let now = chrono::DateTime::parse_from_rfc3339(now).ok()?;
	let until = asked + HEAR_AGAIN_SNOOZE;
	(now < until).then(|| until.with_timezone(&chrono::Utc).to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

/// Where `subject`'s note `note_id` stands with hearing it again at `now`,
/// `keys` being the keys home hears with.
///
/// # Errors
/// Any `sqlx` failure, or a stored verdict this build does not know.
pub async fn hear_again_of(conn: &mut SqliteConnection, subject: &str, note_id: &str, keys: Keys<'_>, now: &str) -> Result<HearAgain, VoiceError> {
	let hearing = hearing(conn, subject, note_id, keys).await?.is_some();
	let request = newest_request(conn, subject, note_id).await?;
	let next_at = request.as_ref().and_then(|request| snoozed_until(&request.asked_at, now));
	let hearable = matches!(hearable(conn, subject, note_id).await?, Some((_, true)));
	let askable = !hearing && next_at.is_none() && hearable && answerable(conn, subject, note_id).await?.is_some();
	Ok(HearAgain {
		open: askable,
		hearing,
		asked_at: request.map(|request| request.asked_at),
		next_at,
	})
}

/// Asks home to hear `subject`'s note `note_id` again at `now`.
///
/// paulgsc/some-ui#3339. `keys` are the keys home hears with. Writes a
/// `hear_again` act answering the verdict [`answerable`] reads, naming the
/// key the note is heard again under, its base key with as many requests as
/// were made under it (`crate::transcript::again`).
///
/// Any verdict may be answered, a heard one included. A resend while home
/// still hears it is answered as it was, and writes nothing. Run it in a
/// `BEGIN IMMEDIATE` transaction.
///
/// # Errors
/// Any `sqlx` failure, or a stored verdict this build does not know.
pub async fn hear_again(conn: &mut SqliteConnection, subject: &str, note_id: &str, keys: Keys<'_>, now: &str) -> Result<Acted, VoiceError> {
	let Some((of_kind, hearable)) = hearable(conn, subject, note_id).await? else {
		return Ok(Acted::NotHeld);
	};
	if let Some(request) = hearing(conn, subject, note_id, keys).await? {
		if let Some(verdict) = answered(conn, subject, note_id, &request).await? {
			return Ok(Acted::Unchanged(verdict));
		}
	}
	let Some(answering) = answerable(conn, subject, note_id).await? else {
		return Ok(Acted::NothingAsked);
	};
	if !hearable {
		return Ok(Acted::NotHearable(answering.verdict));
	}
	if let Some(next_at) = newest_request(conn, subject, note_id).await?.and_then(|request| snoozed_until(&request.asked_at, now)) {
		return Ok(Acted::Snoozed {
			verdict: answering.verdict,
			next_at,
		});
	}
	let base = base_for(&of_kind, keys);
	let asked = sqlx::query_scalar!(
		r#"SELECT hears_under AS "hears_under!" FROM voice_note_attention
		   WHERE subject_id = ?1 AND note_id = ?2 AND act = 'hear_again'"#,
		subject,
		note_id
	)
	.fetch_all(&mut *conn)
	.await?;
	let times = asked.iter().filter(|key| transcript::base_of(key) == base).count();
	let hears_under = transcript::again(base, i64::try_from(times).unwrap_or(i64::MAX - 1) + 1);
	let name = Act::HearAgain.as_str();
	sqlx::query!(
		"INSERT INTO voice_note_attention (subject_id, note_id, n, act, model, attempt, rule, acted_at, hears_under)
		 VALUES (?1, ?2, (SELECT COALESCE(MAX(n), 0) + 1 FROM voice_note_attention WHERE subject_id = ?1 AND note_id = ?2), ?3, ?4, ?5, ?6, ?7, ?8)",
		subject,
		note_id,
		name,
		answering.model,
		answering.attempt,
		answering.verdict.rule,
		now,
		hears_under
	)
	.execute(&mut *conn)
	.await?;
	Ok(Acted::Recorded(answering.verdict))
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
pub async fn needing(conn: &mut SqliteConnection, subject: &str, keys: Keys<'_>, now: &str) -> Result<(Vec<Take>, i64), VoiceError> {
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
		                        AND NOT EXISTS (SELECT 1 FROM voice_note_superseding sup WHERE sup.subject_id = t.subject_id AND sup.note_id = t.note_id AND sup.hears_under <> t.model)
		                      ORDER BY t.outcome = 'heard' DESC, t.heard_at DESC, t.model DESC
		                      LIMIT 1)
		                    OR (NOT EXISTS (
		                          SELECT 1 FROM voice_transcript t
		                          WHERE t.subject_id = n.subject_id AND t.note_id = n.note_id AND t.outcome IN ('heard', 'silent')
		                            AND NOT EXISTS (SELECT 1 FROM voice_note_superseding sup WHERE sup.subject_id = t.subject_id AND sup.note_id = t.note_id AND sup.hears_under <> t.model))
		                        AND v.verdict = 'failed' AND NOT EXISTS (SELECT 1 FROM voice_note_superseding sup WHERE sup.subject_id = v.subject_id AND sup.note_id = v.note_id AND sup.hears_under <> v.model)))
		             ORDER BY v.judged_at DESC, v.rule DESC, v.model DESC, v.attempt DESC
		             LIMIT 1) AS chosen_row
		     FROM voice_note n
		     WHERE n.subject_id = ?1
		       AND EXISTS (
		         SELECT 1 FROM voice_transcript_verdict w
		         WHERE w.subject_id = n.subject_id AND w.note_id = n.note_id AND w.verdict <> 'heard'
		           AND NOT EXISTS (SELECT 1 FROM voice_note_superseding sup WHERE sup.subject_id = w.subject_id AND sup.note_id = w.note_id AND sup.hears_under <> w.model)
		           AND NOT EXISTS (
		             SELECT 1 FROM voice_note_attention a
		             WHERE a.subject_id = w.subject_id AND a.note_id = w.note_id
		               AND a.model = w.model AND a.attempt = w.attempt AND a.rule = w.rule)))
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
		         AND a.model = v.model AND a.attempt = v.attempt AND a.rule = v.rule)
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
		let hear_again = hear_again_of(conn, subject, &row.note_id, keys, now).await?;
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
	/// [`Act::HearAgain`] on a take home would not hear: its recording let
	/// go, or the owner set it aside. Nothing written.
	NotHearable(Verdict),
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
	// A request to hear a take again names the key it is heard under: only
	// [`hear_again`] writes one.
	if act == Act::HearAgain {
		return Ok(Acted::NothingAsked);
	}
	let Some(newest) = answerable(conn, subject, note_id).await? else {
		return Ok(Acted::NothingAsked);
	};
	if !asks(newest.verdict.kind) {
		return Ok(Acted::NothingAsked);
	}
	if answer_of(conn, subject, note_id, &newest).await?.as_deref() == Some(act.as_str()) {
		return Ok(Acted::Unchanged(newest.verdict));
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
pub(crate) mod tests {
	use super::{act, needing, Act, Acted};
	use crate::transcript::{self, Hearing};
	use crate::verdict::{keep, Judged, Kind, Verdict};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";
	const B: &str = "subject-b";
	/// The keys a test's home hears with.
	pub(crate) const KEYS: crate::transcript::Keys<'static> = crate::transcript::Keys {
		plain: "m",
		verbatim: "m+verbatim-1",
	};

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
		let (takes, total) = needing(&mut conn, subject, KEYS, "2026-11-21T00:00:00.000Z").await.unwrap();
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
		let (takes, total) = needing(&mut conn, A, KEYS, "2026-11-21T00:00:00.000Z").await.unwrap();
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
		let (takes, _) = needing(&mut conn, A, KEYS, "2026-11-21T00:00:00.000Z").await.unwrap();
		assert_eq!(takes.first().map(|take| (take.verdict.kind, take.heard.is_none())), Some((Kind::Failed, true)));
		drop(conn);
		heard(&pool, A, "t", "better", Kind::NothingSaid, "2026-11-10").await;
		let mut conn = pool.acquire().await.unwrap();
		let (takes, _) = needing(&mut conn, A, KEYS, "2026-11-21T00:00:00.000Z").await.unwrap();
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
	async fn failed(pool: &SqlitePool, id: &str, model: &str, n: i64, last: bool, at: &str) {
		sqlx::query(
			"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, took_ms, heard_at)
			 VALUES (?1, ?2, ?3, 'e', ?4, 'deadline', 1, ?5)",
		)
		.bind(A)
		.bind(id)
		.bind(model)
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
				model,
				attempt: n,
			};
			let mut conn = pool.acquire().await.unwrap();
			keep(&mut conn, judged, &verdict, at).await.unwrap();
		}
	}

	async fn hear_again_at(pool: &SqlitePool, id: &str, now: &str) -> Acted {
		let mut conn = pool.acquire().await.unwrap();
		super::hear_again(&mut conn, A, id, KEYS, now).await.unwrap()
	}

	async fn standing(pool: &SqlitePool, id: &str, now: &str) -> super::HearAgain {
		let mut conn = pool.acquire().await.unwrap();
		super::hear_again_of(&mut conn, A, id, KEYS, now).await.unwrap()
	}

	/// What waits on model `m`, and the key each is heard under.
	async fn waiting_keys(pool: &SqlitePool) -> Vec<(String, String)> {
		let mut conn = pool.acquire().await.unwrap();
		let keys = crate::transcript::Keys {
			plain: "m",
			verbatim: "m+verbatim-1",
		};
		crate::transcript::waiting(&mut conn, keys, 3, 20)
			.await
			.unwrap()
			.into_iter()
			.map(|waiting| (waiting.note_id, waiting.key))
			.collect()
	}

	fn keyed(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
		pairs.iter().map(|(id, key)| ((*id).to_owned(), (*key).to_owned())).collect()
	}

	async fn audio(pool: &SqlitePool, id: &str) {
		sqlx::query("INSERT INTO voice_note_audio (subject_id, note_id, body) VALUES (?1, ?2, x'00')")
			.bind(A)
			.bind(id)
			.execute(pool)
			.await
			.unwrap();
	}

	/// paulgsc/some-ui#3339: the owner asks home to hear a take again. Each
	/// request is a fresh transcription under a new key, `m+again<k>`, with a
	/// fresh set of attempts; the verdict it answered is skipped while home
	/// hears it, so it leaves the list and reads as not judged yet; the fresh
	/// set's final verdict brings it back; it is asked again only once the
	/// snooze has passed; one home would not hear is refused; no row is
	/// rewritten.
	#[tokio::test]
	async fn a_take_heard_again_is_heard_afresh_under_a_new_key_once_per_snooze() {
		let pool = pool().await;
		note(&pool, A, "t", "studio", "2026-11-01").await;
		audio(&pool, "t").await;
		assert_eq!(waiting_keys(&pool).await, keyed(&[("t", "m")]), "never heard");
		for n in 1..=3 {
			failed(&pool, "t", "m", n, n == 3, "2026-11-20T01:00:00.000Z").await;
		}
		assert!(waiting_keys(&pool).await.is_empty(), "its three attempts spent");
		assert_eq!(listed(&pool, A).await, (vec![String::from("t")], 1));
		let open = standing(&pool, "t", "2026-11-21T00:00:00.000Z").await;
		assert!(open.open && !open.hearing && open.asked_at.is_none() && open.next_at.is_none(), "{open:?}");

		assert!(matches!(hear_again_at(&pool, "t", "2026-11-21T00:00:00.000Z").await, Acted::Recorded(v) if v.kind == Kind::Failed));
		assert!(
			matches!(hear_again_at(&pool, "t", "2026-11-21T00:30:00.000Z").await, Acted::Unchanged(v) if v.kind == Kind::Failed),
			"a resend"
		);
		assert_eq!(waiting_keys(&pool).await, keyed(&[("t", "m+again1")]), "a fresh key, its own three attempts");
		assert_eq!(listed(&pool, A).await, (Vec::new(), 0), "being heard again: off the list");
		let mut conn = pool.acquire().await.unwrap();
		assert!(
			crate::verdict::newest(&mut conn, A, "t").await.unwrap().is_none(),
			"the answered verdict is skipped, not rewritten"
		);
		drop(conn);
		assert_eq!(
			standing(&pool, "t", "2026-11-21T01:00:00.000Z").await,
			super::HearAgain {
				open: false,
				hearing: true,
				asked_at: Some(String::from("2026-11-21T00:00:00.000Z")),
				next_at: Some(String::from("2026-11-21T06:00:00.000Z")),
			}
		);

		// The fresh key fails too: its final verdict brings the take back.
		failed(&pool, "t", "m+again1", 1, false, "2026-11-21T02:00:00.000Z").await;
		assert_eq!(waiting_keys(&pool).await, keyed(&[("t", "m+again1")]), "one of its fresh three used");
		assert!(standing(&pool, "t", "2026-11-21T02:00:00.000Z").await.hearing, "still hearing");
		failed(&pool, "t", "m+again1", 2, false, "2026-11-21T02:10:00.000Z").await;
		failed(&pool, "t", "m+again1", 3, true, "2026-11-21T02:20:00.000Z").await;
		assert!(waiting_keys(&pool).await.is_empty(), "the fresh set spent");
		assert_eq!(listed(&pool, A).await, (vec![String::from("t")], 1), "back, with the new verdict");
		let snoozed = standing(&pool, "t", "2026-11-21T03:00:00.000Z").await;
		assert!(
			!snoozed.open && !snoozed.hearing && snoozed.next_at.as_deref() == Some("2026-11-21T06:00:00.000Z"),
			"{snoozed:?}"
		);
		assert!(matches!(
			hear_again_at(&pool, "t", "2026-11-21T03:00:00.000Z").await,
			Acted::Snoozed { next_at, .. } if next_at == "2026-11-21T06:00:00.000Z"
		));
		// Another act on the new verdict does not stop a later request.
		assert!(matches!(acted(&pool, A, "t", Act::Kept).await, Acted::Recorded(_)));
		assert!(standing(&pool, "t", "2026-11-21T06:00:00.000Z").await.open, "the snooze has passed");
		assert!(matches!(hear_again_at(&pool, "t", "2026-11-21T06:00:00.000Z").await, Acted::Recorded(_)));
		assert_eq!(waiting_keys(&pool).await, keyed(&[("t", "m+again2")]), "a second request, a second fresh key");
		assert!(standing(&pool, "t", "2026-11-21T07:00:00.000Z").await.hearing);

		// Set aside while home hears it: no longer "hearing", and home will
		// not hear it, so it is never asked again.
		sqlx::query("INSERT INTO voice_note_set_aside (subject_id, note_id, set_aside_at, stored_at) VALUES (?1, 't', 'then', 'now')")
			.bind(A)
			.execute(&pool)
			.await
			.unwrap();
		assert!(waiting_keys(&pool).await.is_empty(), "set aside: not heard");
		let aside = standing(&pool, "t", "2026-11-22T00:00:00.000Z").await;
		assert!(!aside.hearing && !aside.open, "{aside:?}");

		// A failed take whose recording was let go is never heard again.
		note(&pool, A, "gone", "studio", "2026-11-03").await;
		failed(&pool, "gone", "m", 1, true, "2026-11-20T01:00:00.000Z").await;
		assert!(!standing(&pool, "gone", "2026-11-21T00:00:00.000Z").await.open, "no recording held");
		assert!(matches!(hear_again_at(&pool, "gone", "2026-11-21T00:00:00.000Z").await, Acted::NotHearable(_)));

		let acts: Vec<(String, String, i64, i64)> = sqlx::query_as("SELECT act, model, attempt, n FROM voice_note_attention WHERE note_id = 't' ORDER BY n")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(
			acts,
			[
				(String::from("hear_again"), String::from("m"), 3, 1),
				(String::from("kept"), String::from("m+again1"), 3, 2),
				(String::from("hear_again"), String::from("m+again1"), 3, 3)
			],
			"one row per act, each naming the verdict it answered"
		);
		let attempts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voice_transcript WHERE note_id = 't'")
			.fetch_one(&pool)
			.await
			.unwrap();
		assert_eq!(attempts, 6, "no attempt rewritten");
	}

	/// paulgsc/some-ui#3339 (C924): a take home heard, a heard one or a
	/// garbled one, may be heard again too, beside its old ending (a model
	/// ends a note once under a key). While home hears it the old words are
	/// skipped, and once the fresh key ends the note, its ending is what is
	/// read, a silence included over the old words.
	#[tokio::test]
	async fn a_heard_take_heard_again_is_read_as_its_fresh_ending() {
		let pool = pool().await;
		note(&pool, A, "h", "studio", "2026-11-01").await;
		audio(&pool, "h").await;
		heard(&pool, A, "h", "m", Kind::Heard, "2026-11-10").await;
		let mut conn = pool.acquire().await.unwrap();
		assert!(matches!(transcript::heard(&mut conn, A, "h").await.unwrap(), Hearing::Heard(_)));
		drop(conn);
		assert!(standing(&pool, "h", "2026-11-21T00:00:00.000Z").await.open, "a heard take may be heard again");
		assert!(matches!(hear_again_at(&pool, "h", "2026-11-21T00:00:00.000Z").await, Acted::Recorded(v) if v.kind == Kind::Heard));
		assert_eq!(waiting_keys(&pool).await, keyed(&[("h", "m+again1")]));
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(
			transcript::heard(&mut conn, A, "h").await.unwrap(),
			Hearing::Waiting,
			"the old words skipped while home hears it"
		);
		assert!(transcript::words(&mut conn, A, "h").await.unwrap().is_empty());
		drop(conn);

		heard(&pool, A, "h", "m+again1", Kind::NothingSaid, "2026-11-21T01:00:00.000Z").await;
		assert!(waiting_keys(&pool).await.is_empty(), "the fresh key ended it");
		let mut conn = pool.acquire().await.unwrap();
		match transcript::heard(&mut conn, A, "h").await.unwrap() {
			Hearing::Heard(fresh) => assert_eq!((fresh.text.as_str(), fresh.model.as_str()), ("", "m+again1"), "its silence, over the old words"),
			other => panic!("{other:?}"),
		}
		let newest = crate::verdict::newest(&mut conn, A, "h").await.unwrap().unwrap();
		assert_eq!((newest.model.as_str(), newest.verdict.kind), ("m+again1", Kind::NothingSaid));
		drop(conn);
		assert_eq!(listed(&pool, A).await, (vec![String::from("h")], 1), "now it needs attention");
		let old: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voice_transcript WHERE note_id = 'h' AND model = 'm'")
			.fetch_one(&pool)
			.await
			.unwrap();
		assert_eq!(old, 1, "the old ending kept");
	}

	/// Asking home to hear a take again never makes it undeletable: while
	/// home hears it, the delete rule reads the verdict the request answered
	/// (a garbled reflection, deletable as one home could not hear), and My
	/// data's `deletable` agrees.
	#[tokio::test]
	async fn a_take_being_heard_again_can_still_be_deleted() {
		let pool = pool().await;
		note(&pool, A, "r", "reflection", "2026-11-01").await;
		audio(&pool, "r").await;
		heard(&pool, A, "r", "m", Kind::Garbled, "2026-11-10").await;
		assert!(matches!(hear_again_at(&pool, "r", "2026-11-21T00:00:00.000Z").await, Acted::Recorded(_)));
		let mut conn = pool.acquire().await.unwrap();
		assert!(crate::verdict::newest(&mut conn, A, "r").await.unwrap().is_none(), "being heard again");
		assert_eq!(super::asked_of(&mut conn, A, "r").await.unwrap(), Some(Kind::Garbled));
		let kept = crate::kept::newest(&mut conn, A, crate::Of::Reflection, crate::delete::SWITCHED_ON, None).await.unwrap();
		assert!(kept.iter().any(|note| note.id == "r" && note.deletable), "My data says it may go");
		drop(conn);
		let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
		let deleted = crate::delete::delete(&mut tx, A, "r", crate::delete::SWITCHED_ON, "2026-11-21T01:00:00.000Z")
			.await
			.unwrap();
		tx.commit().await.unwrap();
		assert!(matches!(deleted, crate::delete::Deleted::Gone(_)), "{deleted:?}");
		let acts: Vec<String> = sqlx::query_scalar("SELECT act FROM voice_note_attention WHERE note_id = 'r' ORDER BY n")
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(acts, ["hear_again", "deleted"], "the delete is recorded as an act, answering what the request answered");
	}

	/// The review of be9410a: a take with endings under two models (before
	/// and after a change of model) is heard again under the current model's
	/// fresh key, whichever model's verdict the request answered; every other
	/// key's ending is skipped while home hears it, so nothing shows through,
	/// and the fresh key's silence is read over the older model's words; a
	/// mark set aside ends the supersession, and the old ending reads again.
	#[tokio::test]
	async fn a_take_heard_by_two_models_is_heard_again_under_the_current_ones_fresh_key() {
		let pool = pool().await;
		note(&pool, A, "t", "studio", "2026-11-01").await;
		audio(&pool, "t").await;
		heard(&pool, A, "t", "old", Kind::Garbled, "2026-11-05").await;
		for n in 1..=3 {
			failed(&pool, "t", "m", n, n == 3, "2026-11-20T01:00:00.000Z").await;
		}
		let mut conn = pool.acquire().await.unwrap();
		let newest = crate::verdict::newest(&mut conn, A, "t").await.unwrap().unwrap();
		assert_eq!(newest.model, "old", "the older model's ending is the one read");
		drop(conn);
		assert!(matches!(hear_again_at(&pool, "t", "2026-11-21T00:00:00.000Z").await, Acted::Recorded(v) if v.kind == Kind::Garbled));
		assert_eq!(
			waiting_keys(&pool).await,
			keyed(&[("t", "m+again1")]),
			"the current model's fresh key, not the answered model's"
		);
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(transcript::heard(&mut conn, A, "t").await.unwrap(), Hearing::Waiting, "no other key shows through");
		assert!(crate::verdict::newest(&mut conn, A, "t").await.unwrap().is_none());
		drop(conn);
		assert!(standing(&pool, "t", "2026-11-21T01:00:00.000Z").await.hearing);
		assert_eq!(listed(&pool, A).await, (Vec::new(), 0), "not back on the list mid-hearing");

		heard(&pool, A, "t", "m+again1", Kind::NothingSaid, "2026-11-21T02:00:00.000Z").await;
		let mut conn = pool.acquire().await.unwrap();
		match transcript::heard(&mut conn, A, "t").await.unwrap() {
			Hearing::Heard(fresh) => assert_eq!(fresh.model, "m+again1", "the fresh silence, over the older model's words"),
			other => panic!("{other:?}"),
		}
		drop(conn);
		assert!(!standing(&pool, "t", "2026-11-21T03:00:00.000Z").await.hearing);

		// Set aside: the supersession ends, and every key reads as before.
		sqlx::query("INSERT INTO voice_note_set_aside (subject_id, note_id, set_aside_at, stored_at) VALUES (?1, 't', 'then', 'now')")
			.bind(A)
			.execute(&pool)
			.await
			.unwrap();
		let mut conn = pool.acquire().await.unwrap();
		match transcript::heard(&mut conn, A, "t").await.unwrap() {
			Hearing::Heard(read) => assert_eq!(read.model, "old", "the older model's words read again"),
			other => panic!("{other:?}"),
		}
	}

	/// A request named under a model home no longer hears with is moot: the
	/// take is not "hearing", and the owner may ask again under the new one.
	#[tokio::test]
	async fn a_request_made_under_an_old_model_is_moot_and_may_be_asked_again() {
		let pool = pool().await;
		note(&pool, A, "t", "studio", "2026-11-01").await;
		audio(&pool, "t").await;
		for n in 1..=3 {
			failed(&pool, "t", "old", n, n == 3, "2026-11-20T01:00:00.000Z").await;
		}
		let old = crate::transcript::Keys {
			plain: "old",
			verbatim: "old+verbatim-1",
		};
		let mut conn = pool.acquire().await.unwrap();
		assert!(matches!(
			super::hear_again(&mut conn, A, "t", old, "2026-11-01T00:00:00.000Z").await.unwrap(),
			Acted::Recorded(_)
		));
		assert!(super::hear_again_of(&mut conn, A, "t", old, "2026-11-01T01:00:00.000Z").await.unwrap().hearing);
		let moot = super::hear_again_of(&mut conn, A, "t", KEYS, "2026-11-21T00:00:00.000Z").await.unwrap();
		assert!(!moot.hearing && moot.open, "under the new model: moot, and open {moot:?}");
		assert!(matches!(
			super::hear_again(&mut conn, A, "t", KEYS, "2026-11-21T00:00:00.000Z").await.unwrap(),
			Acted::Recorded(_)
		));
		drop(conn);
		assert_eq!(waiting_keys(&pool).await, keyed(&[("t", "m+again1")]));
	}

	#[test]
	fn a_fresh_key_is_its_base_and_the_times_asked() {
		assert_eq!(transcript::again("ggml-small-q5_1.bin", 0), "ggml-small-q5_1.bin");
		assert_eq!(transcript::again("ggml-small-q5_1.bin", 2), "ggml-small-q5_1.bin+again2");
		assert_eq!(transcript::base_of("ggml-small-q5_1.bin+verbatim-1+again1"), "ggml-small-q5_1.bin+verbatim-1");
		assert_eq!(transcript::base_of("ggml-small-q5_1.bin"), "ggml-small-q5_1.bin");
		assert_eq!(transcript::base_of("m+again"), "m+again", "no number: not a fresh key");
	}
}
