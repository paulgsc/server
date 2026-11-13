//! What the owner did with a week of Rust Rounds (RR14).
//!
//! paulgsc/some-ui#2689: the week as Send kept it on the phone, and the errors pasted in it.
//! `migrations/20261109000100_create_rust_week_record.up.sql` records the
//! design; in short:
//!
//! - **The phone's `WeekSent`, checked against its week** ([`read_sent`]):
//!   every ticket of the week once, answered with a take or passed over, at
//!   least one answered, sent no later than the week's `due` and no earlier
//!   than it `opens`, and reaching home no later than `due` either. A take not sent by the deadline doesn't count (owner,
//!   2026-11-07), so a later `at` is refused, not kept as late.
//! - **A draft until the deadline, final at it** (RR22,
//!   paulgsc/some-ui#2832; `migrations/20261110000100_create_rust_week_draft_final.up.sql`):
//!   before the week's `due`, each send replaces the last as the week's draft
//!   ([`put_draft`]). A send is named by its `at`, compared as an instant: the
//!   same value again changes nothing, another body at the draft's moment is
//!   refused ([`RoundError::Rewritten`]), and an older one leaves the draft
//!   as it is. Once home's clock is past `due` no send is taken
//!   ([`read_sent`]), so the draft standing then is final whenever it is
//!   written: [`finalize`] writes it to `rust_week_final` and drops the
//!   draft, in one transaction. No final is ever updated or, while the
//!   account stands, deleted (the migration's triggers).
//! - **RR14's `rust_week_sent` is an archive**: its rows were copied as their
//!   weeks' drafts, and nothing writes it now.
//! - **A take is named by its voice note's id, never held here.** Its
//!   audio is a voice note (docs/identity.md invariant 23), which may reach
//!   home before or after the week it answers.
//! - **A paste is the owner's own code's error, as pasted** ([`read_paste`],
//!   [`put_paste`]): kept once per week and moment, and never refused for
//!   the deadline: a paste is not a take.
//! - **Per subject.** Every query takes the subject and filters on it.

use crate::draft::{self, Kind, Stored, WeekSend};
use crate::round::Problem;
use crate::week::{Week, Window};
use crate::{instant_order, Change, RoundError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqliteConnection;
use std::cmp::Ordering;

pub use crate::draft::DraftChange;

/// The most bytes a week sent may hold: four tickets' answers are well
/// under a KiB.
pub const SENT_CEILING: usize = 16 * 1024;

/// The longest paste, in characters: the phone's `PASTE_MAX`, which counts
/// UTF-16 units, so every paste it keeps is at most this many characters.
pub const PASTE_MAX_CHARS: usize = 20_000;

/// The most bytes a paste's body may hold: its text at four bytes a
/// character, and room for the rest.
pub const PASTE_CEILING: usize = PASTE_MAX_CHARS * 4 + 1024;

/// How far ahead of home's clock a phone's moment may be: its clock may run
/// a little fast, never a day.
const SKEW_MINUTES: i64 = 5;

/// [`RoundError::Rewritten`]'s words for a week sent, for the `409`.
pub const SENT_TWICE: &str = "home holds another send of this week at this moment; a send is named by its moment, and the phone never reuses one";

/// [`RoundError::Rewritten`]'s words for a paste, for the `409`.
pub const PASTED_TWICE: &str = "home holds another paste at this moment of this week; a paste is kept as it was";

/// One ticket answered, as Send kept it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Answer {
	/// The ticket's key, `QM-311`.
	pub ticket: String,
	/// The take's voice-note id.
	pub take: String,
	/// When the take was recorded.
	pub recorded_at: String,
}

/// A week's answers as sent: the phone's `WeekSent`
/// (paulgsc/some-ui `packages/ui/rust-rounds/src/lib/week-play/spec.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sent {
	/// The week's id.
	pub week: String,
	/// When Send was pressed.
	pub at: String,
	/// Each ticket said, in the week's order.
	pub answers: Vec<Answer>,
	/// The tickets not said, in the week's order.
	pub passed: Vec<String>,
}

impl WeekSend for Sent {
	const KIND: Kind = Kind::RustRounds;

	fn week(&self) -> &str {
		&self.week
	}

	fn at(&self) -> &str {
		&self.at
	}
}

/// A week sent, as stored: its draft or its final.
pub type StoredSent = Stored<Sent>;

/// An error pasted in a week, as the phone keeps it (its `KeptPaste`, less
/// the week, which the route names).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Paste {
	/// When the paste was opened: names it within its week.
	pub at: String,
	/// The error, as pasted.
	pub text: String,
	/// The voice note said on it, by id.
	pub note: Option<String>,
}

/// A paste, as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredPaste {
	pub at: String,
	pub text: String,
	pub note: Option<String>,
	pub received_at: String,
}

/// A voice note's id as home names one (`voice_repo::is_note_id`): 1 to 64
/// letters, digits, `.`, `_`, `:` or `-`.
#[must_use]
pub fn is_note_id(id: &str) -> bool {
	(1..=64).contains(&id.len()) && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

pub(crate) fn is_instant(at: &str) -> bool {
	chrono::DateTime::parse_from_rfc3339(at).is_ok()
}

/// Whether `at` is more than a few minutes past `now`.
fn is_ahead(at: &str, now: &str) -> bool {
	let (Ok(at), Ok(now)) = (chrono::DateTime::parse_from_rfc3339(at), chrono::DateTime::parse_from_rfc3339(now)) else {
		return false;
	};
	at > now + chrono::Duration::minutes(SKEW_MINUTES)
}

pub(crate) fn json_object(body: &[u8], ceiling: usize, ceiling_words: &'static str) -> Result<Value, Vec<Problem>> {
	let problem = |field: &str, why| vec![(field.to_owned(), why)];
	if body.len() > ceiling {
		return Err(problem("body", ceiling_words));
	}
	match serde_json::from_slice::<Value>(body) {
		Ok(value @ Value::Object(_)) => Ok(value),
		_ => Err(problem("body", "is not one JSON object")),
	}
}

/// The problems with a send pressed at `at`, of a week open over `window`,
/// reaching home at `now`, whatever kind of week it is: `at` not an instant,
/// before the week opens, after its deadline or ahead of `now`; a week with
/// no deadline; or home's clock past the deadline (the week is final).
pub(crate) fn sent_in_window(at: &str, window: Option<&Window>, now: &str, problems: &mut Vec<Problem>) {
	match window {
		_ if !is_instant(at) => problems.push((String::from("at"), "must be an RFC 3339 instant")),
		Some(window) if instant_order(at, &window.due) == Some(Ordering::Greater) => {
			problems.push((String::from("at"), "is after the week's deadline: a take not sent by then doesn't count"));
		}
		Some(window) if instant_order(at, &window.opens) == Some(Ordering::Less) => {
			problems.push((String::from("at"), "is before the week opens"));
		}
		None => problems.push((String::from("week"), "has no deadline home can read")),
		Some(_) if is_ahead(at, now) => problems.push((String::from("at"), "is ahead of home's clock")),
		Some(_) => {}
	}
	if window.is_some_and(|window| instant_order(now, &window.due) == Some(Ordering::Greater)) {
		problems.push((String::from("week"), "is final: its deadline has passed, and what stood then is the record"));
	}
}

/// Read `body` as `week`'s answers, sent at a moment home's clock reads as
/// `now`, or say every problem found.
///
/// # Errors
/// Each [`Problem`]: a body past [`SENT_CEILING`], not one JSON object, or
/// not the format's shape (an unknown key included); a `week` that is not
/// `week`'s id, or is final (`now` is past its deadline); an `at` that is not an instant, is before the week opens,
/// after its deadline or ahead of `now`; no answer; a ticket the week
/// does not name, or named twice, or not named at all; a take that is not
/// a note id; a `recordedAt` that is not an instant or is after `at`.
pub fn read_sent(body: &[u8], week: &Week, now: &str) -> Result<Sent, Vec<Problem>> {
	let value = json_object(body, SENT_CEILING, "is larger than 16 KiB")?;
	let sent: Sent = serde_json::from_value(value).map_err(|_| vec![(String::from("body"), "must be a week sent: week, at, answers and passed, and nothing else")])?;
	let mut problems: Vec<Problem> = Vec::new();
	if sent.week != week.id {
		problems.push((String::from("week"), "must be the week in the path"));
	}
	sent_in_window(&sent.at, week.window.as_ref(), now, &mut problems);
	if sent.answers.is_empty() {
		problems.push((String::from("answers"), "must hold at least one ticket answered"));
	}
	let tickets: Vec<&str> = week.programs.iter().map(|program| program.ticket.as_str()).collect();
	let named: Vec<&str> = sent
		.answers
		.iter()
		.map(|answer| answer.ticket.as_str())
		.chain(sent.passed.iter().map(String::as_str))
		.collect();
	if named.iter().any(|ticket| !tickets.contains(ticket)) {
		problems.push((String::from("answers"), "names a ticket the week does not have"));
	}
	if tickets.iter().any(|ticket| named.iter().filter(|each| *each == ticket).count() != 1) {
		problems.push((String::from("passed"), "with the answers, must name each of the week's tickets once"));
	}
	for (index, answer) in sent.answers.iter().enumerate() {
		let at = |field: &str| String::from("answers[") + &index.to_string() + "]." + field;
		if !is_note_id(&answer.take) {
			problems.push((at("take"), "must be a voice note's id"));
		}
		if !is_instant(&answer.recorded_at) {
			problems.push((at("recordedAt"), "must be an RFC 3339 instant"));
		} else if instant_order(&answer.recorded_at, &sent.at) == Some(Ordering::Greater) {
			problems.push((at("recordedAt"), "must not be after the week was sent"));
		}
	}
	if problems.is_empty() {
		Ok(sent)
	} else {
		Err(problems)
	}
}

/// Keep `sent` as `subject`'s draft of its week ([`draft::put_draft`]).
///
/// `sent` is read by [`read_sent`] with home's clock no later than the
/// week's deadline. It says what was done, and the draft that stands after
/// it, by the rules every kind of week shares.
///
/// # Errors
/// As [`draft::put_draft`].
pub async fn put_draft(conn: &mut SqliteConnection, subject: &str, sent: &Sent, now: &str) -> Result<(DraftChange, Sent), RoundError> {
	draft::put_draft(conn, subject, sent, now).await
}

/// Whether `week` is final for `subject` ([`draft::is_final`]).
///
/// # Errors
/// Any `sqlx` failure.
pub async fn is_final(conn: &mut SqliteConnection, subject: &str, week: &str) -> Result<bool, sqlx::Error> {
	draft::is_final(conn, Kind::RustRounds, subject, week).await
}

/// `subject`'s send of `week` that stands: its final once it has one, else
/// its draft ([`draft::standing`]).
///
/// # Errors
/// Any `sqlx` failure, or a stored body that no longer reads as one.
pub async fn standing(conn: &mut SqliteConnection, subject: &str, week: &str) -> Result<Option<StoredSent>, sqlx::Error> {
	draft::standing(conn, subject, week).await
}

/// The weeks `subject` holds a draft of ([`draft::drafted_weeks`]).
///
/// # Errors
/// Any `sqlx` failure.
pub async fn drafted_weeks(conn: &mut SqliteConnection, subject: &str) -> Result<Vec<String>, sqlx::Error> {
	draft::drafted_weeks(conn, Kind::RustRounds, subject).await
}

/// Make `week`'s standing draft its final once home's clock is past `due`;
/// whether it wrote one ([`draft::finalize`]).
///
/// # Errors
/// Any `sqlx` failure. Run it in a `BEGIN IMMEDIATE` transaction.
pub async fn finalize(conn: &mut SqliteConnection, subject: &str, week: &str, due: &str, now: &str) -> Result<bool, sqlx::Error> {
	draft::finalize(conn, Kind::RustRounds, subject, week, due, now).await
}

/// Read `body` as a paste made at `at`, the path's moment, with home's
/// clock at `now`, or say every problem found.
///
/// # Errors
/// Each [`Problem`]: a body past [`PASTE_CEILING`], not one JSON object or
/// not the format's shape; an `at` that is not an instant, not the path's,
/// or ahead of `now`; empty text or text past [`PASTE_MAX_CHARS`]; a note
/// that is not a note id.
pub fn read_paste(body: &[u8], at: &str, now: &str) -> Result<Paste, Vec<Problem>> {
	let value = json_object(body, PASTE_CEILING, "is larger than a paste may be")?;
	let paste: Paste = serde_json::from_value(value).map_err(|_| vec![(String::from("body"), "must be a paste: at, text and note, and nothing else")])?;
	let mut problems: Vec<Problem> = Vec::new();
	if !is_instant(&paste.at) {
		problems.push((String::from("at"), "must be an RFC 3339 instant"));
	} else if paste.at != at {
		problems.push((String::from("at"), "must be the moment in the path"));
	} else if is_ahead(&paste.at, now) {
		problems.push((String::from("at"), "is ahead of home's clock"));
	}
	if paste.text.is_empty() {
		problems.push((String::from("text"), "must hold the error pasted"));
	} else if paste.text.chars().count() > PASTE_MAX_CHARS {
		problems.push((String::from("text"), "is longer than 20,000 characters"));
	}
	if paste.note.as_deref().is_some_and(|note| !is_note_id(note)) {
		problems.push((String::from("note"), "must be a voice note's id, or null"));
	}
	if problems.is_empty() {
		Ok(paste)
	} else {
		Err(problems)
	}
}

/// Store `paste`, made in `week`, for `subject`, or nothing when the same is
/// held.
///
/// # Errors
/// [`RoundError::Rewritten`] for another paste at the same moment of the
/// week, which writes nothing; [`RoundError::Storage`] for any `sqlx`
/// failure.
pub async fn put_paste(conn: &mut SqliteConnection, subject: &str, week: &str, paste: &Paste, now: &str) -> Result<Change, RoundError> {
	let held = sqlx::query!(
		"SELECT text, note_id FROM rust_week_paste WHERE subject_id = ?1 AND week_id = ?2 AND at = ?3",
		subject,
		week,
		paste.at
	)
	.fetch_optional(&mut *conn)
	.await?;
	match held {
		Some(held) if (held.text.as_str(), held.note_id.as_deref()) == (paste.text.as_str(), paste.note.as_deref()) => Ok(Change::Unchanged),
		Some(_) => Err(RoundError::Rewritten),
		None => {
			sqlx::query!(
				"INSERT INTO rust_week_paste (subject_id, week_id, at, text, note_id, received_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
				subject,
				week,
				paste.at,
				paste.text,
				paste.note,
				now
			)
			.execute(&mut *conn)
			.await?;
			Ok(Change::Added)
		}
	}
}

/// `subject`'s pastes in `week`, in the order they were made: by the instant
/// each `at` names, whatever its offset, so read and sorted here rather than
/// by the stored text. A week's pastes are a handful.
///
/// # Errors
/// Any `sqlx` failure.
pub async fn pastes(conn: &mut SqliteConnection, subject: &str, week: &str) -> Result<Vec<StoredPaste>, sqlx::Error> {
	let mut pastes = sqlx::query_as!(
		StoredPaste,
		"SELECT at, text, note_id AS note, received_at FROM rust_week_paste WHERE subject_id = ?1 AND week_id = ?2",
		subject,
		week
	)
	.fetch_all(&mut *conn)
	.await?;
	pastes.sort_by(|a, b| (crate::time_order_key(&a.at), &a.at).cmp(&(crate::time_order_key(&b.at), &b.at)));
	Ok(pastes)
}

#[cfg(test)]
mod tests {
	use super::{finalize, is_final, pastes, put_draft, put_paste, read_paste, read_sent, standing, DraftChange, Sent};
	use crate::week::read;
	use crate::{Change, RoundError};
	use serde_json::{json, Value};
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";
	const B: &str = "subject-b";
	const NOW: &str = "2026-11-16T09:00:00.000Z";
	/// Home's clock at the week's deadline, 23:59 Pacific: the last moment
	/// a send is taken.
	const OPEN: &str = "2026-11-16T07:59:00.000Z";
	const DUE: &str = "2026-11-15T23:59:00-08:00";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		for subject in [A, B] {
			sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?1)")
				.bind(subject)
				.execute(&pool)
				.await
				.unwrap();
		}
		pool
	}

	fn week() -> crate::week::Week {
		let ticket = |id: &str| json!({ "id": id, "program": "fn main() {}\n" });
		let body = json!({
			"schema": "some-ui.rust-week/1", "id": "2026-w46",
			"opens": "2026-11-09T00:00:00-08:00", "due": "2026-11-15T23:59:00-08:00",
			"tickets": [ticket("QM-311"), ticket("QM-312"), ticket("QM-313")],
		});
		read(body.to_string().as_bytes()).unwrap()
	}

	fn sent_body(at: &str) -> Value {
		json!({
			"week": "2026-w46", "at": at,
			"answers": [
				{ "ticket": "QM-311", "take": "take-1", "recordedAt": "2026-11-12T20:00:00.000Z" },
				{ "ticket": "QM-313", "take": "take-3", "recordedAt": "2026-11-12T20:05:00.000Z" },
			],
			"passed": ["QM-312"],
		})
	}

	fn fields(problems: &[(String, &'static str)]) -> Vec<String> {
		let mut fields: Vec<String> = problems.iter().map(|(field, _)| field.clone()).collect();
		fields.sort();
		fields.dedup();
		fields
	}

	/// The deadline is the week's, offset and all: a Send a minute before it
	/// counts, a minute after does not, and one before the week opens or
	/// ahead of home's clock is no Send.
	#[test]
	fn a_week_is_sent_between_its_opening_and_its_deadline() {
		let week = week();
		assert!(
			read_sent(sent_body("2026-11-16T07:58:00.000Z").to_string().as_bytes(), &week, OPEN).is_ok(),
			"23:58 Pacific"
		);
		for at in ["2026-11-16T08:00:00.000Z", "2026-11-09T07:59:00.000Z", "2026-11-15T23:59:00-08:01"] {
			let mut body = sent_body(at);
			for answer in body["answers"].as_array_mut().unwrap() {
				answer["recordedAt"] = json!("2026-11-01T00:00:00.000Z");
			}
			let refused = read_sent(body.to_string().as_bytes(), &week, OPEN).unwrap_err();
			assert_eq!(fields(&refused), ["at"], "{at}");
		}
		let final_now = read_sent(sent_body("2026-11-15T20:00:00.000Z").to_string().as_bytes(), &week, NOW).unwrap_err();
		assert_eq!(fields(&final_now), ["week"], "home's clock past the deadline: the week is final, whenever it was pressed");
		let early_now = "2026-11-12T21:00:00.000Z";
		let refused = read_sent(sent_body("2026-11-12T21:10:00.000Z").to_string().as_bytes(), &week, early_now).unwrap_err();
		assert_eq!(fields(&refused), ["at"], "ahead of home's clock");
		assert!(
			read_sent(sent_body("2026-11-12T21:04:00.000Z").to_string().as_bytes(), &week, early_now).is_ok(),
			"a few minutes fast is fine"
		);
	}

	/// Every ticket of the week is answered or passed, once; at least one is
	/// answered; a take is a note id said before Send; nothing else is kept.
	#[test]
	fn a_week_sent_names_each_ticket_once_and_nothing_else() {
		let week = week();
		let at = "2026-11-15T20:00:00.000Z";
		let refused = |body: Value| fields(&read_sent(body.to_string().as_bytes(), &week, OPEN).unwrap_err());
		let mut missing = sent_body(at);
		missing["passed"] = json!([]);
		assert_eq!(refused(missing), ["passed"]);
		let mut twice = sent_body(at);
		twice["passed"] = json!(["QM-312", "QM-311"]);
		assert_eq!(refused(twice), ["passed"]);
		let mut stranger = sent_body(at);
		stranger["passed"] = json!(["QM-312", "QM-999"]);
		assert_eq!(refused(stranger), ["answers"]);
		let none = json!({ "week": "2026-w46", "at": at, "answers": [], "passed": ["QM-311", "QM-312", "QM-313"] });
		assert_eq!(refused(none), ["answers"]);
		let mut bad_take = sent_body(at);
		bad_take["answers"][0]["take"] = json!("a take/with a slash");
		bad_take["answers"][1]["recordedAt"] = json!("2026-11-15T21:00:00.000Z");
		assert_eq!(refused(bad_take), ["answers[0].take", "answers[1].recordedAt"]);
		let mut other_week = sent_body(at);
		other_week["week"] = json!("2026-w47");
		assert_eq!(refused(other_week), ["week"]);
		let mut extra = sent_body(at);
		extra["score"] = json!(3);
		assert_eq!(refused(extra), ["body"]);
		let mut extra_in_answer = sent_body(at);
		extra_in_answer["answers"][0]["words"] = json!("what I said");
		assert_eq!(refused(extra_in_answer), ["body"]);
	}

	/// Before the deadline a send replaces the draft when it is later, leaves
	/// it when it is earlier, and is the same send at the draft's moment only
	/// as the same value; each subject drafts its own.
	#[tokio::test]
	async fn a_later_send_replaces_the_draft() {
		let pool = pool().await;
		let week = week();
		let read = |at: &str| -> Sent { read_sent(sent_body(at).to_string().as_bytes(), &week, OPEN).unwrap() };
		let (first, later) = (read("2026-11-15T20:00:00.000Z"), read("2026-11-15T21:00:00.000Z"));
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(put_draft(&mut conn, A, &first, OPEN).await.unwrap(), (DraftChange::Added, first.clone()));
		assert_eq!(put_draft(&mut conn, A, &first, OPEN).await.unwrap(), (DraftChange::Unchanged, first.clone()));
		let reordered = r#"{ "passed": ["QM-312"],
			"answers": [ { "recordedAt": "2026-11-12T20:00:00.000Z", "take": "take-1", "ticket": "QM-311" },
			             { "take": "take-3", "ticket": "QM-313", "recordedAt": "2026-11-12T20:05:00.000Z" } ],
			"at": "2026-11-15T12:00:00.000-08:00", "week": "2026-w46" }"#;
		let respelled = read_sent(reordered.as_bytes(), &week, OPEN).unwrap();
		assert!(
			matches!(put_draft(&mut conn, A, &respelled, OPEN).await, Err(RoundError::Rewritten)),
			"the same instant written otherwise is another body at the draft's moment"
		);
		let reordered = reordered.replace("2026-11-15T12:00:00.000-08:00", "2026-11-15T20:00:00.000Z");
		let same = read_sent(reordered.as_bytes(), &week, OPEN).unwrap();
		assert_eq!(
			put_draft(&mut conn, A, &same, OPEN).await.unwrap().0,
			DraftChange::Unchanged,
			"the same value, not the same bytes"
		);
		let mut changed = first.clone();
		changed.answers[0].take = String::from("take-1-redo");
		assert!(matches!(put_draft(&mut conn, A, &changed, OPEN).await, Err(RoundError::Rewritten)));
		assert_eq!(put_draft(&mut conn, A, &later, OPEN).await.unwrap(), (DraftChange::Replaced, later.clone()));
		assert_eq!(
			put_draft(&mut conn, A, &first, OPEN).await.unwrap(),
			(DraftChange::Superseded, later.clone()),
			"a late retry"
		);
		let held = standing(&mut conn, A, "2026-w46").await.unwrap().unwrap();
		assert_eq!((held.sent, held.draft), (later.clone(), true));
		assert!(standing(&mut conn, B, "2026-w46").await.unwrap().is_none(), "another subject's draft is not there");
		assert_eq!(put_draft(&mut conn, B, &first, OPEN).await.unwrap().0, DraftChange::Added, "nor in the way");
	}

	/// The draft standing at the deadline is the final, written once and the
	/// same whenever home first reads the week after it; before the deadline
	/// nothing is final, and a week with no draft has no final.
	#[tokio::test]
	async fn the_draft_standing_at_the_deadline_is_final() {
		let pool = pool().await;
		let week = week();
		let mut conn = pool.acquire().await.unwrap();
		let sent = read_sent(sent_body("2026-11-15T21:00:00.000Z").to_string().as_bytes(), &week, OPEN).unwrap();
		put_draft(&mut conn, A, &sent, OPEN).await.unwrap();
		assert!(!finalize(&mut conn, A, "2026-w46", DUE, OPEN).await.unwrap(), "not before the deadline");
		assert!(!is_final(&mut conn, A, "2026-w46").await.unwrap());
		assert!(standing(&mut conn, A, "2026-w46").await.unwrap().unwrap().draft);
		assert!(finalize(&mut conn, A, "2026-w46", DUE, NOW).await.unwrap());
		let held = standing(&mut conn, A, "2026-w46").await.unwrap().unwrap();
		assert_eq!((&held.sent, held.draft), (&sent, false));
		assert!(is_final(&mut conn, A, "2026-w46").await.unwrap());
		assert!(!is_final(&mut conn, B, "2026-w46").await.unwrap(), "another subject's week is its own");
		let drafts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rust_week_draft").fetch_one(&mut *conn).await.unwrap();
		assert_eq!(drafts, 0, "the draft went with the transaction that made it final");
		// Were a draft ever beside the final, the final is what stands.
		let other = read_sent(sent_body("2026-11-15T22:00:00.000Z").to_string().as_bytes(), &week, OPEN).unwrap();
		put_draft(&mut conn, A, &other, OPEN).await.unwrap();
		assert_eq!(standing(&mut conn, A, "2026-w46").await.unwrap().unwrap(), held);
		sqlx::query("DELETE FROM rust_week_draft").execute(&mut *conn).await.unwrap();
		assert!(!finalize(&mut conn, A, "2026-w46", DUE, "2026-12-01T00:00:00Z").await.unwrap(), "once");
		assert_eq!(standing(&mut conn, A, "2026-w46").await.unwrap().unwrap(), held);
		assert!(!finalize(&mut conn, B, "2026-w46", DUE, NOW).await.unwrap(), "nothing sent: no final");
		assert!(standing(&mut conn, B, "2026-w46").await.unwrap().is_none());
	}

	/// RR14's rows are left as they were, and each is its week's standing
	/// draft after the migration: a later send replaces the copy, never the
	/// row.
	#[tokio::test]
	async fn a_week_rr14_kept_is_its_first_draft_and_stays_as_it_was() {
		const RR22: i64 = 20_261_110_000_100;
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		let ups = || MIGRATOR.iter().filter(|migration| migration.migration_type.is_up_migration());
		for migration in ups().filter(|migration| migration.version < RR22) {
			sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
		}
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?1)")
			.bind(A)
			.execute(&pool)
			.await
			.unwrap();
		// As RR14's put_sent stored it: the struct's own order, no spaces.
		let held = r#"{"week":"2026-w46","at":"2026-11-15T20:00:00.000Z","answers":[{"ticket":"QM-311","take":"take-1","recordedAt":"2026-11-12T20:00:00.000Z"},{"ticket":"QM-313","take":"take-3","recordedAt":"2026-11-12T20:05:00.000Z"}],"passed":["QM-312"]}"#;
		sqlx::query(
			"INSERT INTO rust_week_sent (subject_id, week_id, sent_at, body, received_at) VALUES (?1, '2026-w46', '2026-11-15T20:00:00.000Z', ?2, '2026-11-15T20:00:01.000Z')",
		)
		.bind(A)
		.bind(held)
		.execute(&pool)
		.await
		.unwrap();
		for migration in ups().filter(|migration| migration.version >= RR22) {
			sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
		}
		let week = week();
		let mut conn = pool.acquire().await.unwrap();
		let draft = standing(&mut conn, A, "2026-w46").await.unwrap().unwrap();
		assert_eq!((draft.text.as_str(), draft.draft, draft.received_at.as_str()), (held, true, "2026-11-15T20:00:01.000Z"));
		let again = read_sent(serde_json::from_str::<Value>(held).unwrap().to_string().as_bytes(), &week, OPEN).unwrap();
		assert_eq!(put_draft(&mut conn, A, &again, OPEN).await.unwrap().0, DraftChange::Unchanged, "the phone re-sending it");
		let later = read_sent(sent_body("2026-11-15T21:00:00.000Z").to_string().as_bytes(), &week, OPEN).unwrap();
		assert_eq!(put_draft(&mut conn, A, &later, OPEN).await.unwrap().0, DraftChange::Replaced);
		let archive: (String, String) = sqlx::query_as("SELECT sent_at, body FROM rust_week_sent WHERE subject_id = ?1")
			.bind(A)
			.fetch_one(&mut *conn)
			.await
			.unwrap();
		assert_eq!(archive, (String::from("2026-11-15T20:00:00.000Z"), String::from(held)), "RR14's row as it was");
	}

	/// A paste is kept once per moment of its week, for any moment; a
	/// different one at the same moment is refused.
	#[tokio::test]
	async fn a_paste_is_kept_once_and_read_back_in_order() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		let body = |at: &str, text: &str| json!({ "at": at, "text": text, "note": null }).to_string();
		let late = read_paste(body("2026-11-16T08:30:00.000Z", "error[E0502]").as_bytes(), "2026-11-16T08:30:00.000Z", NOW).unwrap();
		let early = read_paste(body("2026-11-10T08:30:00.000Z", "error[E0499]").as_bytes(), "2026-11-10T08:30:00.000Z", NOW).unwrap();
		assert_eq!(
			put_paste(&mut conn, A, "2026-w46", &late, NOW).await.unwrap(),
			Change::Added,
			"a paste after the deadline is kept"
		);
		assert_eq!(put_paste(&mut conn, A, "2026-w46", &early, NOW).await.unwrap(), Change::Added);
		assert_eq!(put_paste(&mut conn, A, "2026-w46", &early, NOW).await.unwrap(), Change::Unchanged);
		let mut changed = early.clone();
		changed.note = Some(String::from("2026-w46:PASTE:2026-11-10T08:30:00.000Z"));
		assert!(matches!(put_paste(&mut conn, A, "2026-w46", &changed, NOW).await, Err(RoundError::Rewritten)));
		let held: Vec<String> = pastes(&mut conn, A, "2026-w46").await.unwrap().into_iter().map(|paste| paste.text).collect();
		assert_eq!(held, ["error[E0499]", "error[E0502]"]);
		// 10:00 at +02:00 is 08:00 UTC: before 09:00Z, though its text sorts after.
		let offset = read_paste(body("2026-11-11T10:00:00+02:00", "east").as_bytes(), "2026-11-11T10:00:00+02:00", NOW).unwrap();
		let utc = read_paste(body("2026-11-11T09:00:00Z", "utc").as_bytes(), "2026-11-11T09:00:00Z", NOW).unwrap();
		put_paste(&mut conn, B, "2026-w47", &utc, NOW).await.unwrap();
		put_paste(&mut conn, B, "2026-w47", &offset, NOW).await.unwrap();
		let held: Vec<String> = pastes(&mut conn, B, "2026-w47").await.unwrap().into_iter().map(|paste| paste.text).collect();
		assert_eq!(held, ["east", "utc"], "in time, not by text");
		assert!(pastes(&mut conn, B, "2026-w46").await.unwrap().is_empty());
	}

	#[test]
	fn a_paste_names_its_moment_and_holds_text() {
		let check = |body: Value, at: &str| read_paste(body.to_string().as_bytes(), at, NOW).map_err(|problems| fields(&problems));
		let at = "2026-11-10T08:30:00.000Z";
		assert_eq!(check(json!({ "at": at, "text": "", "note": null }), at).unwrap_err(), ["text"]);
		assert_eq!(check(json!({ "at": at, "text": "x".repeat(20_001), "note": null }), at).unwrap_err(), ["text"]);
		assert!(check(json!({ "at": at, "text": "é".repeat(20_000), "note": null }), at).is_ok(), "counted in characters");
		assert_eq!(check(json!({ "at": at, "text": "e", "note": "no spaces allowed" }), at).unwrap_err(), ["note"]);
		assert_eq!(check(json!({ "at": at, "text": "e", "note": null }), "2026-11-10T08:31:00.000Z").unwrap_err(), ["at"]);
		assert_eq!(check(json!({ "at": at, "text": "e", "note": null, "week": "2026-w46" }), at).unwrap_err(), ["body"]);
	}

	/// No row is ever updated, and none deleted while its account stands;
	/// the account takes them all.
	#[tokio::test]
	async fn the_record_refuses_an_update_and_a_delete_and_leaves_with_the_account() {
		let pool = pool().await;
		let week = week();
		let mut conn = pool.acquire().await.unwrap();
		let first = read_sent(sent_body("2026-11-15T20:00:00.000Z").to_string().as_bytes(), &week, OPEN).unwrap();
		put_draft(&mut conn, A, &first, OPEN).await.unwrap();
		assert!(finalize(&mut conn, A, "2026-w46", DUE, NOW).await.unwrap());
		let other = read_sent(sent_body("2026-11-15T20:00:00.000Z").to_string().as_bytes(), &week, OPEN).unwrap();
		// A draft beside the final, as no send could make one: a row to see
		// leave with the account.
		put_draft(&mut conn, A, &other, OPEN).await.unwrap();
		sqlx::query("INSERT INTO rust_week_sent (subject_id, week_id, sent_at, body, received_at) VALUES (?1, '2026-w45', 'then', '{}', 'then')")
			.bind(A)
			.execute(&mut *conn)
			.await
			.unwrap();
		let paste = read_paste(json!({ "at": NOW, "text": "e", "note": null }).to_string().as_bytes(), NOW, NOW).unwrap();
		put_paste(&mut conn, A, "2026-w46", &paste, NOW).await.unwrap();
		for statement in [
			"UPDATE rust_week_sent SET sent_at = 'x'",
			"DELETE FROM rust_week_sent",
			"UPDATE rust_week_final SET sent_at = 'x'",
			"DELETE FROM rust_week_final",
			"UPDATE rust_week_paste SET text = 'x'",
			"DELETE FROM rust_week_paste",
		] {
			assert!(sqlx::query(statement).execute(&mut *conn).await.is_err(), "{statement}");
		}
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&mut *conn).await.unwrap();
		for table in ["rust_week_sent", "rust_week_draft", "rust_week_final", "rust_week_paste"] {
			let left: i64 = sqlx::query_scalar(&(String::from("SELECT COUNT(*) FROM ") + table)).fetch_one(&mut *conn).await.unwrap();
			assert_eq!(left, 0, "{table}");
		}
	}
}
