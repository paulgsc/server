//! What home reads of a week of mock interview, and what the owner sent of
//! it (paulgsc/some-ui#2935, IW4, under epic #2931).
//!
//! The format is paulgsc/some-ui's `some-ui.interview-week/1`, written by the
//! Monday routine and held to all of it there before it is put in the
//! mailbox, as `interview-week-<id>.json` in `brief` ([`MAILBOX_PREFIX`]), the
//! way a week of Rust Rounds rides (`crate::week`). Home checks only what it
//! keeps the owner's sends against ([`read`]):
//!
//! - its size ([`WEEK_CEILING`]), its encoding and that it is one JSON object;
//! - its `schema` tag;
//! - its `id`, the ISO week it opens (`2026-w47`), which names it for good;
//! - each question's `id` and each teleprompter card's `id`, slugs
//!   (`crate::round::is_slug`), each once, no question named `read`, so
//!   every take's ref (`<week>:<question>`, `<week>:read:<card>`,
//!   `voice_repo::is_interview_ref`) names one;
//! - its `opens` and `due`, RFC 3339 instants, the deadline after the
//!   opening. A week without them is still kept, and takes no send.
//!
//! Anything else (the invented company and role, each question's family,
//! prompt, target length and guidelines, each card's source and script) is
//! kept as it came and never read here. Unlike Rust Rounds' week, nothing is
//! queued: a week of interview runs no program.
//!
//! **What the owner sent** ([`read_sent`], [`Sent`]): each question answered
//! and each card read, with its take (a voice note, by id, never audio) and
//! when it was recorded, and those passed over. It is a draft until the
//! week's deadline and final at it, by the rules every kind of week shares
//! (`crate::draft`, kept in `interview_week_draft` and
//! `interview_week_final`).

use crate::draft::{Kind, WeekSend};
use crate::instant_order;
use crate::round::{is_slug, Problem};
use crate::sent::{is_instant, is_note_id, json_object, sent_in_window, SENT_CEILING};
use crate::week::{is_week_id, Window};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;

/// The format's tag.
pub const WEEK_SCHEMA: &str = "some-ui.interview-week/1";

/// The most bytes a week may hold.
///
/// Four to six questions, each with its prompt and guidelines, and a few
/// teleprompter cards of 150 to 300 words each: well under this.
pub const WEEK_CEILING: usize = 64 * 1024;

/// What a week's name in the mailbox starts with. It rides `brief`, as a
/// week of Rust Rounds does (`crate::week::MAILBOX_PREFIX`).
pub const MAILBOX_PREFIX: &str = "interview-week-";

/// What no question is named: the second part of a read's ref.
const READ: &str = "read";

/// The mailbox name a week is put under: `interview-week-<id>.json`.
#[must_use]
pub fn mailbox_name(id: &str) -> String {
	String::from(MAILBOX_PREFIX) + id + ".json"
}

/// A week home can keep sends of: its id, its questions' and cards' ids in
/// the week's order, and its window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Week {
	pub id: String,
	pub questions: Vec<String>,
	pub cards: Vec<String>,
	/// When the week opens and its deadline, when both are RFC 3339
	/// instants and the deadline comes after the opening; `None` otherwise.
	pub window: Option<Window>,
}

/// The ids of `list` (a week's `questions` or `cards`), each a slug and
/// used once, with each problem found under `field`.
fn ids(list: &[Value], field: &str, problems: &mut Vec<Problem>) -> Vec<String> {
	let mut held: Vec<String> = Vec::new();
	for (index, item) in list.iter().enumerate() {
		let at = String::from(field) + "[" + &index.to_string() + "].id";
		let Some(id) = item.get("id").and_then(Value::as_str).filter(|id| is_slug(id)) else {
			problems.push((at, "must be a slug like about-you, at most 64 characters"));
			continue;
		};
		if field == "questions" && id == READ {
			problems.push((at, "must not be read, which names a card's take"));
		} else if held.iter().any(|each| each == id) {
			problems.push((at, "is used by an earlier one"));
		} else {
			held.push(id.to_owned());
		}
	}
	held
}

/// Read `body` as a week of mock interview, or say every problem found.
///
/// # Errors
/// Each [`Problem`]: a body past [`WEEK_CEILING`], not UTF-8 or not one JSON
/// object; a missing or other `schema`; an `id` that is not an ISO week; no
/// `questions`; `cards` that is there and not a list; a question or a card
/// whose id is not a slug or is used twice, or a question named `read`.
pub fn read(body: &[u8]) -> Result<Week, Vec<Problem>> {
	let problem = |field: &str, why| vec![(field.to_owned(), why)];
	if body.len() > WEEK_CEILING {
		return Err(problem("body", "is larger than 64 KiB"));
	}
	let Ok(text) = std::str::from_utf8(body) else {
		return Err(problem("body", "is not UTF-8"));
	};
	let Ok(Value::Object(week)) = serde_json::from_str::<Value>(text) else {
		return Err(problem("body", "is not one JSON object"));
	};
	let mut problems = Vec::new();
	if week.get("schema").and_then(Value::as_str) != Some(WEEK_SCHEMA) {
		problems.push((String::from("schema"), "must be \"some-ui.interview-week/1\""));
	}
	let id = week.get("id").and_then(Value::as_str).filter(|id| is_week_id(id));
	if id.is_none() {
		problems.push((String::from("id"), "must be an ISO year and week, like 2026-w47"));
	}
	let questions = match week.get("questions").and_then(Value::as_array) {
		Some(list) if !list.is_empty() => ids(list, "questions", &mut problems),
		_ => {
			problems.push((String::from("questions"), "must hold at least one question"));
			Vec::new()
		}
	};
	let cards = match week.get("cards") {
		None => Vec::new(),
		Some(Value::Array(list)) => ids(list, "cards", &mut problems),
		Some(_) => {
			problems.push((String::from("cards"), "must be a list of cards"));
			Vec::new()
		}
	};
	let instant = |field: &str| week.get(field).and_then(Value::as_str).filter(|at| is_instant(at)).map(str::to_owned);
	let window = match (instant("opens"), instant("due")) {
		(Some(opens), Some(due)) if instant_order(&opens, &due) == Some(Ordering::Less) => Some(Window { opens, due }),
		_ => None,
	};
	match id {
		Some(id) if problems.is_empty() => Ok(Week {
			id: id.to_owned(),
			questions,
			cards,
			window,
		}),
		_ => Err(problems),
	}
}

/// One question answered, as Send kept it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Answer {
	/// The question's id.
	pub question: String,
	/// The take's voice-note id.
	pub take: String,
	/// When the take was recorded.
	pub recorded_at: String,
}

/// One teleprompter card read, as Send kept it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Read {
	/// The card's id.
	pub card: String,
	/// The take's voice-note id.
	pub take: String,
	/// When the take was recorded.
	pub recorded_at: String,
}

/// A week's takes as sent: the phone's send of a week of mock interview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Sent {
	/// The week's id.
	pub week: String,
	/// When Send was pressed.
	pub at: String,
	/// Each question answered, in the week's order.
	pub answers: Vec<Answer>,
	/// Each card read, in the week's order.
	pub reads: Vec<Read>,
	/// The questions not answered, in the week's order.
	pub passed: Vec<String>,
	/// The cards not read, in the week's order.
	pub passed_reads: Vec<String>,
}

impl WeekSend for Sent {
	const KIND: Kind = Kind::Interview;

	fn week(&self) -> &str {
		&self.week
	}

	fn at(&self) -> &str {
		&self.at
	}
}

/// The problems with `taken` and `passed` against the week's `ids`: each
/// named once between them, and nothing the week does not have.
fn names_each_once(ids: &[String], taken: &[&str], passed: &[String], (field, passed_field): (&str, &str), problems: &mut Vec<Problem>) {
	let named: Vec<&str> = taken.iter().copied().chain(passed.iter().map(String::as_str)).collect();
	if named.iter().any(|id| !ids.iter().any(|each| each == id)) {
		problems.push((field.to_owned(), "names one the week does not have"));
	}
	if ids.iter().any(|id| named.iter().filter(|each| **each == id).count() != 1) {
		problems.push((passed_field.to_owned(), "with those taken, must name each of the week's once"));
	}
}

/// The problems with one take: its id a note's, recorded no later than the
/// send.
fn take_problems(field: &str, (take, recorded_at): (&str, &str), at: &str, problems: &mut Vec<Problem>) {
	if !is_note_id(take) {
		problems.push((String::from(field) + ".take", "must be a voice note's id"));
	}
	if !is_instant(recorded_at) {
		problems.push((String::from(field) + ".recordedAt", "must be an RFC 3339 instant"));
	} else if instant_order(recorded_at, at) == Some(Ordering::Greater) {
		problems.push((String::from(field) + ".recordedAt", "must not be after the week was sent"));
	}
}

/// Read `body` as `week`'s takes, sent at a moment home's clock reads as
/// `now`, or say every problem found.
///
/// # Errors
/// Each [`Problem`]: a body past [`SENT_CEILING`], not one JSON object, or
/// not the format's shape (an unknown key included); a `week` that is not
/// `week`'s id, or is final (`now` is past its deadline); an `at` that is not
/// an instant, is before the week opens, after its deadline or ahead of
/// `now`; no take at all; a question or a card the week does not name, named
/// twice, or not named at all; a take that is not a note id; a `recordedAt`
/// that is not an instant or is after `at`.
pub fn read_sent(body: &[u8], week: &Week, now: &str) -> Result<Sent, Vec<Problem>> {
	let value = json_object(body, SENT_CEILING, "is larger than 16 KiB")?;
	let sent: Sent = serde_json::from_value(value).map_err(|_| {
		vec![(
			String::from("body"),
			"must be a week sent: week, at, answers, reads, passed and passedReads, and nothing else",
		)]
	})?;
	let mut problems: Vec<Problem> = Vec::new();
	if sent.week != week.id {
		problems.push((String::from("week"), "must be the week in the path"));
	}
	sent_in_window(&sent.at, week.window.as_ref(), now, &mut problems);
	if sent.answers.is_empty() && sent.reads.is_empty() {
		problems.push((String::from("answers"), "with the reads, must hold at least one take"));
	}
	let answered: Vec<&str> = sent.answers.iter().map(|answer| answer.question.as_str()).collect();
	names_each_once(&week.questions, &answered, &sent.passed, ("answers", "passed"), &mut problems);
	let read: Vec<&str> = sent.reads.iter().map(|read| read.card.as_str()).collect();
	names_each_once(&week.cards, &read, &sent.passed_reads, ("reads", "passedReads"), &mut problems);
	for (index, answer) in sent.answers.iter().enumerate() {
		let field = String::from("answers[") + &index.to_string() + "]";
		take_problems(&field, (&answer.take, &answer.recorded_at), &sent.at, &mut problems);
	}
	for (index, read) in sent.reads.iter().enumerate() {
		let field = String::from("reads[") + &index.to_string() + "]";
		take_problems(&field, (&read.take, &read.recorded_at), &sent.at, &mut problems);
	}
	if problems.is_empty() {
		Ok(sent)
	} else {
		Err(problems)
	}
}

#[cfg(test)]
mod tests {
	use super::{mailbox_name, read, read_sent, Sent};
	use crate::draft::{finalize, is_final, put_draft, standing, DraftChange, Kind, Stored};
	use serde_json::{json, Value};
	use sqlx::sqlite::SqlitePoolOptions;

	const OPEN: &str = "2026-11-17T09:00:00.000Z";
	const DUE: &str = "2026-11-22T23:59:00-08:00";
	const AFTER: &str = "2026-11-23T09:00:00.000Z";

	fn week_body() -> Value {
		json!({
			"schema": "some-ui.interview-week/1", "id": "2026-w47",
			"opens": "2026-11-16T00:00:00-08:00", "due": DUE,
			"company": { "name": "Fernbrook Labs" }, "role": "Platform engineer",
			"questions": [
				{ "id": "about-you", "family": "about-you", "prompt": "Tell me about yourself.", "targetSeconds": 90 },
				{ "id": "a-conflict", "family": "a-time", "prompt": "A time you disagreed.", "targetSeconds": 120 },
			],
			"cards": [{ "id": "home-first", "script": "I built home first because…", "targetSeconds": 90 }],
		})
	}

	fn sent_body(at: &str) -> Value {
		json!({
			"week": "2026-w47", "at": at,
			"answers": [{ "question": "about-you", "take": "n1", "recordedAt": "2026-11-17T08:00:00.000Z" }],
			"reads": [{ "card": "home-first", "take": "n2", "recordedAt": "2026-11-17T08:30:00.000Z" }],
			"passed": ["a-conflict"], "passedReads": [],
		})
	}

	fn fields(problems: &[(String, &'static str)]) -> Vec<String> {
		problems.iter().map(|(field, _)| field.clone()).collect()
	}

	#[test]
	fn a_week_gives_its_questions_cards_and_window() {
		let week = read(week_body().to_string().as_bytes()).unwrap();
		assert_eq!(week.id, "2026-w47");
		assert_eq!(week.questions, ["about-you", "a-conflict"]);
		assert_eq!(week.cards, ["home-first"]);
		assert_eq!(week.window.unwrap().due, DUE);
		assert_eq!(mailbox_name("2026-w47"), "interview-week-2026-w47.json");
		let mut no_cards = week_body();
		no_cards.as_object_mut().unwrap().remove("cards");
		assert!(read(no_cards.to_string().as_bytes()).unwrap().cards.is_empty(), "a week may have no cards");
	}

	#[test]
	fn a_week_home_cannot_keep_says_every_problem() {
		let body = json!({ "schema": "some-ui.rust-week/1", "id": "week-47",
			"questions": [{ "id": "About" }, { "id": "read" }, { "id": "x" }, { "id": "x" }],
			"cards": [{ "id": "c" }, { "id": "c" }] });
		assert_eq!(
			fields(&read(body.to_string().as_bytes()).unwrap_err()),
			["schema", "id", "questions[0].id", "questions[1].id", "questions[3].id", "cards[1].id"]
		);
		assert_eq!(
			fields(
				&read(
					json!({ "schema": "some-ui.interview-week/1", "id": "2026-w47", "questions": [], "cards": 1 })
						.to_string()
						.as_bytes()
				)
				.unwrap_err()
			),
			["questions", "cards"]
		);
		assert_eq!(fields(&read(&vec![b' '; super::WEEK_CEILING + 1]).unwrap_err()), ["body"]);
	}

	#[test]
	fn a_send_names_each_question_and_card_once_within_the_window() {
		let week = read(week_body().to_string().as_bytes()).unwrap();
		let sent = read_sent(sent_body(OPEN).to_string().as_bytes(), &week, OPEN).unwrap();
		assert_eq!((sent.answers.len(), sent.reads.len()), (1, 1));
		let bad = json!({
			"week": "2026-w46", "at": "2026-11-23T08:00:00.000Z",
			"answers": [{ "question": "nope", "take": "a b", "recordedAt": "2026-11-24T00:00:00.000Z" }],
			"reads": [], "passed": ["about-you", "about-you"], "passedReads": [],
		});
		assert_eq!(
			fields(&read_sent(bad.to_string().as_bytes(), &week, OPEN).unwrap_err()),
			["week", "at", "answers", "passed", "passedReads", "answers[0].take", "answers[0].recordedAt"]
		);
		let none = json!({ "week": "2026-w47", "at": OPEN, "answers": [], "reads": [], "passed": ["about-you", "a-conflict"], "passedReads": ["home-first"] });
		assert_eq!(fields(&read_sent(none.to_string().as_bytes(), &week, OPEN).unwrap_err()), ["answers"], "at least one take");
		assert_eq!(
			fields(&read_sent(sent_body(OPEN).to_string().as_bytes(), &week, AFTER).unwrap_err()),
			["week"],
			"final once home's clock is past due"
		);
		assert_eq!(fields(&read_sent(br#"{"week":"2026-w47","extra":1}"#, &week, OPEN).unwrap_err()), ["body"]);
	}

	/// The draft and the final keep Rust Rounds' rules, in the interview's
	/// own tables: a later send replaces the draft, and what stands at the
	/// deadline is final, once.
	#[tokio::test]
	async fn a_send_is_a_draft_until_the_deadline_then_final() {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		sqlx::migrate!("../../../migrations").run(&pool).await.unwrap();
		let mut conn = pool.acquire().await.unwrap();
		let week = read(week_body().to_string().as_bytes()).unwrap();
		let first = read_sent(sent_body(OPEN).to_string().as_bytes(), &week, OPEN).unwrap();
		let later = read_sent(sent_body("2026-11-18T09:00:00.000Z").to_string().as_bytes(), &week, "2026-11-18T09:00:00.000Z").unwrap();
		assert_eq!(put_draft(&mut conn, "a", &first, OPEN).await.unwrap().0, DraftChange::Added);
		assert_eq!(put_draft(&mut conn, "a", &later, OPEN).await.unwrap().0, DraftChange::Replaced);
		assert_eq!(put_draft(&mut conn, "a", &first, OPEN).await.unwrap(), (DraftChange::Superseded, later.clone()));
		let rust: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rust_week_draft").fetch_one(&mut *conn).await.unwrap();
		assert_eq!(rust, 0, "an interview send is not a Rust Rounds draft");
		assert!(!finalize(&mut conn, Kind::Interview, "a", "2026-w47", DUE, OPEN).await.unwrap());
		assert!(
			!finalize(&mut conn, Kind::RustRounds, "a", "2026-w47", DUE, AFTER).await.unwrap(),
			"the other kind holds none"
		);
		assert!(finalize(&mut conn, Kind::Interview, "a", "2026-w47", DUE, AFTER).await.unwrap());
		assert!(is_final(&mut conn, Kind::Interview, "a", "2026-w47").await.unwrap());
		let held: Stored<Sent> = standing(&mut conn, "a", "2026-w47").await.unwrap().unwrap();
		assert_eq!((held.sent, held.draft), (later, false));
		assert!(!finalize(&mut conn, Kind::Interview, "a", "2026-w47", DUE, AFTER).await.unwrap(), "once");
		// A draft that no longer reads as a send is left a draft: no final, no error.
		sqlx::query("INSERT INTO interview_week_draft (subject_id, week_id, sent_at, body, received_at) VALUES ('a', '2026-w46', ?1, '{\"week\":1}', ?1)")
			.bind(OPEN)
			.execute(&mut *conn)
			.await
			.unwrap();
		assert!(!finalize(&mut conn, Kind::Interview, "a", "2026-w46", DUE, AFTER).await.unwrap());
		assert!(!is_final(&mut conn, Kind::Interview, "a", "2026-w46").await.unwrap(), "left a draft");
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES ('a', 'a')")
			.execute(&mut *conn)
			.await
			.unwrap();
		for statement in ["UPDATE interview_week_final SET sent_at = 'x'", "DELETE FROM interview_week_final"] {
			assert!(sqlx::query(statement).execute(&mut *conn).await.is_err(), "{statement}");
		}
	}
}
