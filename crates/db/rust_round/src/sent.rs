//! What the owner did with a week of Rust Rounds (RR14).
//!
//! paulgsc/some-ui#2689: the week as Send kept it on the phone, and the errors pasted in it.
//! `migrations/20261109000100_create_rust_week_record.up.sql` records the
//! design; in short:
//!
//! - **The phone's `WeekSent`, checked against its week** ([`read_sent`]):
//!   every ticket of the week once, answered with a take or passed over, at
//!   least one answered, sent no later than the week's `due` and no earlier
//!   than it `opens`. A take not sent by the deadline doesn't count (owner,
//!   2026-11-07), so a later `at` is refused, not kept as late.
//! - **Sent once** ([`put_sent`]): the same body again changes nothing;
//!   another is refused ([`RoundError::Rewritten`]). No row is ever updated
//!   or, while the account stands, deleted (the migration's triggers).
//! - **A take is named by its voice note's id, never held here.** Its
//!   audio is a voice note (docs/identity.md invariant 23), which may reach
//!   home before or after the week it answers.
//! - **A paste is the owner's own code's error, as pasted** ([`read_paste`],
//!   [`put_paste`]): kept once per week and moment, and never refused for
//!   the deadline: a paste is not a take.
//! - **Per subject.** Every query takes the subject and filters on it.

use crate::round::Problem;
use crate::week::Week;
use crate::{instant_order, Change, RoundError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqliteConnection;
use std::cmp::Ordering;

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
pub const SENT_TWICE: &str = "home holds another Send of this week; a week is sent once, and what was sent is kept as it was";

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

/// A week sent, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSent {
	pub sent: Sent,
	/// The body as stored, re-serialised from [`Sent`].
	pub text: String,
	pub received_at: String,
}

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

fn is_instant(at: &str) -> bool {
	chrono::DateTime::parse_from_rfc3339(at).is_ok()
}

/// Whether `at` is more than a few minutes past `now`.
fn is_ahead(at: &str, now: &str) -> bool {
	let (Ok(at), Ok(now)) = (chrono::DateTime::parse_from_rfc3339(at), chrono::DateTime::parse_from_rfc3339(now)) else {
		return false;
	};
	at > now + chrono::Duration::minutes(SKEW_MINUTES)
}

fn json_object(body: &[u8], ceiling: usize, ceiling_words: &'static str) -> Result<Value, Vec<Problem>> {
	let problem = |field: &str, why| vec![(field.to_owned(), why)];
	if body.len() > ceiling {
		return Err(problem("body", ceiling_words));
	}
	match serde_json::from_slice::<Value>(body) {
		Ok(value @ Value::Object(_)) => Ok(value),
		_ => Err(problem("body", "is not one JSON object")),
	}
}

/// Read `body` as `week`'s answers, sent at a moment home's clock reads as
/// `now`, or say every problem found.
///
/// # Errors
/// Each [`Problem`]: a body past [`SENT_CEILING`], not one JSON object, or
/// not the format's shape (an unknown key included); a `week` that is not
/// `week`'s id; an `at` that is not an instant, is before the week opens,
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
	match &week.window {
		_ if !is_instant(&sent.at) => problems.push((String::from("at"), "must be an RFC 3339 instant")),
		Some(window) if instant_order(&sent.at, &window.due) == Some(Ordering::Greater) => {
			problems.push((String::from("at"), "is after the week's deadline: a take not sent by then doesn't count"));
		}
		Some(window) if instant_order(&sent.at, &window.opens) == Some(Ordering::Less) => {
			problems.push((String::from("at"), "is before the week opens"));
		}
		None => problems.push((String::from("week"), "has no deadline home can read")),
		Some(_) if is_ahead(&sent.at, now) => problems.push((String::from("at"), "is ahead of home's clock")),
		Some(_) => {}
	}
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

/// Store `sent` for `subject`, or nothing when the same is held.
///
/// # Errors
/// [`RoundError::Rewritten`] for another body under the week, which writes
/// nothing; [`RoundError::Storage`] for any `sqlx` failure.
pub async fn put_sent(conn: &mut SqliteConnection, subject: &str, sent: &Sent, now: &str) -> Result<Change, RoundError> {
	// Re-serialising a checked value: its own field names and the phone's
	// strings, nothing more.
	#[allow(clippy::disallowed_methods)]
	let text = serde_json::to_string(sent).map_err(|err| RoundError::Storage(sqlx::Error::Decode(Box::new(err))))?;
	let held = sqlx::query_scalar!("SELECT body FROM rust_week_sent WHERE subject_id = ?1 AND week_id = ?2", subject, sent.week)
		.fetch_optional(&mut *conn)
		.await?;
	match held {
		Some(held) if held == text => Ok(Change::Unchanged),
		Some(_) => Err(RoundError::Rewritten),
		None => {
			sqlx::query!(
				"INSERT INTO rust_week_sent (subject_id, week_id, sent_at, body, received_at) VALUES (?1, ?2, ?3, ?4, ?5)",
				subject,
				sent.week,
				sent.at,
				text,
				now
			)
			.execute(&mut *conn)
			.await?;
			Ok(Change::Added)
		}
	}
}

/// `subject`'s Send of `week`, if home holds one.
///
/// # Errors
/// Any `sqlx` failure, or a stored body that no longer reads as one.
pub async fn sent(conn: &mut SqliteConnection, subject: &str, week: &str) -> Result<Option<StoredSent>, sqlx::Error> {
	let row = sqlx::query!("SELECT body, received_at FROM rust_week_sent WHERE subject_id = ?1 AND week_id = ?2", subject, week)
		.fetch_optional(&mut *conn)
		.await?;
	row
		.map(|row| {
			let sent = serde_json::from_str(&row.body).map_err(|err| sqlx::Error::Decode(Box::new(err)))?;
			Ok(StoredSent {
				sent,
				text: row.body,
				received_at: row.received_at,
			})
		})
		.transpose()
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
	use super::{pastes, put_paste, put_sent, read_paste, read_sent, sent, Sent};
	use crate::week::read;
	use crate::{Change, RoundError};
	use serde_json::{json, Value};
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const A: &str = "subject-a";
	const B: &str = "subject-b";
	const NOW: &str = "2026-11-16T09:00:00.000Z";

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
		assert!(read_sent(sent_body("2026-11-16T07:58:00.000Z").to_string().as_bytes(), &week, NOW).is_ok(), "23:58 Pacific");
		for at in ["2026-11-16T08:00:00.000Z", "2026-11-09T07:59:00.000Z", "2026-11-15T23:59:00-08:01"] {
			let mut body = sent_body(at);
			for answer in body["answers"].as_array_mut().unwrap() {
				answer["recordedAt"] = json!("2026-11-01T00:00:00.000Z");
			}
			let refused = read_sent(body.to_string().as_bytes(), &week, NOW).unwrap_err();
			assert_eq!(fields(&refused), ["at"], "{at}");
		}
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
		let refused = |body: Value| fields(&read_sent(body.to_string().as_bytes(), &week, NOW).unwrap_err());
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

	/// A week is sent once: the same Send again changes nothing, another is
	/// refused and the first stays; each subject holds its own.
	#[tokio::test]
	async fn a_week_is_sent_once_and_never_replaced() {
		let pool = pool().await;
		let week = week();
		let first: Sent = read_sent(sent_body("2026-11-15T20:00:00.000Z").to_string().as_bytes(), &week, NOW).unwrap();
		let other: Sent = read_sent(sent_body("2026-11-15T20:01:00.000Z").to_string().as_bytes(), &week, NOW).unwrap();
		let mut conn = pool.acquire().await.unwrap();
		assert_eq!(put_sent(&mut conn, A, &first, NOW).await.unwrap(), Change::Added);
		assert_eq!(put_sent(&mut conn, A, &first, NOW).await.unwrap(), Change::Unchanged);
		assert!(matches!(put_sent(&mut conn, A, &other, NOW).await, Err(RoundError::Rewritten)));
		assert_eq!(sent(&mut conn, A, "2026-w46").await.unwrap().unwrap().sent, first);
		assert!(sent(&mut conn, B, "2026-w46").await.unwrap().is_none(), "another subject's Send is not there");
		assert_eq!(put_sent(&mut conn, B, &other, NOW).await.unwrap(), Change::Added, "nor in the way");
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
		let first = read_sent(sent_body("2026-11-15T20:00:00.000Z").to_string().as_bytes(), &week, NOW).unwrap();
		put_sent(&mut conn, A, &first, NOW).await.unwrap();
		let paste = read_paste(json!({ "at": NOW, "text": "e", "note": null }).to_string().as_bytes(), NOW, NOW).unwrap();
		put_paste(&mut conn, A, "2026-w46", &paste, NOW).await.unwrap();
		for statement in [
			"UPDATE rust_week_sent SET sent_at = 'x'",
			"DELETE FROM rust_week_sent",
			"UPDATE rust_week_paste SET text = 'x'",
			"DELETE FROM rust_week_paste",
		] {
			assert!(sqlx::query(statement).execute(&mut *conn).await.is_err(), "{statement}");
		}
		sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&mut *conn).await.unwrap();
		for table in ["rust_week_sent", "rust_week_paste"] {
			let left: i64 = sqlx::query_scalar(&(String::from("SELECT COUNT(*) FROM ") + table)).fetch_one(&mut *conn).await.unwrap();
			assert_eq!(left, 0, "{table}");
		}
	}
}
