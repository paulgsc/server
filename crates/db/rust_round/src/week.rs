//! What home reads of a week of Rust Rounds: the least it needs to keep it
//! and to record its programs (paulgsc/some-ui#2687, RR12).
//!
//! The format is paulgsc/some-ui's `some-ui.rust-week/1`
//! (`packages/ui/rust-rounds/src/lib/formats/week.ts`, RR11), and the
//! routine holds a week to all of it (`readWeek`) before it puts one in the
//! mailbox, as `rust-week-<id>.json` in `brief` ([`MAILBOX_PREFIX`]). Home
//! checks only:
//!
//! - its size ([`WEEK_CEILING`]), its encoding and that it is one JSON object;
//! - its `schema` tag;
//! - its `id`, the ISO week it opens (`2026-w46`), which names it for good;
//! - its tickets' ids and every ticket's `program`, the bytes it records.
//!
//! Anything else (the company, the story, the speak-to points, the tutor's
//! guess) is kept as it came and never read here.

use crate::round::Problem;
use serde_json::Value;

/// The format's tag.
pub const WEEK_SCHEMA: &str = "some-ui.rust-week/1";

/// The most bytes a week may hold: a round's ceiling.
///
/// A week is two to four tickets, each program at most 25 lines of 40
/// columns, and their prose: a few KiB, so each program fits the runner's
/// request with room.
pub const WEEK_CEILING: usize = crate::round::ROUND_CEILING;

/// One ticket's program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketProgram {
	/// The ticket's key, like `QM-311`.
	pub ticket: String,
	/// `main.rs`, exactly.
	pub source: String,
}

/// A week home can keep: its id and its tickets' programs, in ticket order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Week {
	pub id: String,
	pub programs: Vec<TicketProgram>,
}

/// What a week's name in the mailbox starts with.
///
/// A week rides the `brief` folder (paulgsc/some-ui `apps/mailbox`, `also`),
/// which home lists on every pass anyway, so it adds no request to a pass
/// with nothing to do (MI6).
pub const MAILBOX_PREFIX: &str = "rust-week-";

/// The mailbox name a week is put under: `rust-week-<id>.json`.
#[must_use]
pub fn mailbox_name(id: &str) -> String {
	String::from(MAILBOX_PREFIX) + id + ".json"
}

/// Read `body` as a week, or say every problem found.
///
/// # Errors
/// Each [`Problem`]: a body past [`WEEK_CEILING`], not UTF-8 or not one JSON
/// object; a missing or other `schema`; an `id` that is not an ISO week; no
/// `tickets`; a ticket whose id is not a key or is used twice, or that has
/// no `program`.
pub fn read(body: &[u8]) -> Result<Week, Vec<Problem>> {
	let problem = |field: &str, why| vec![(field.to_owned(), why)];
	if body.len() > WEEK_CEILING {
		return Err(problem("body", "is larger than 32 KiB"));
	}
	let Ok(text) = std::str::from_utf8(body) else {
		return Err(problem("body", "is not UTF-8"));
	};
	let Ok(Value::Object(week)) = serde_json::from_str::<Value>(text) else {
		return Err(problem("body", "is not one JSON object"));
	};
	let mut problems = Vec::new();
	if week.get("schema").and_then(Value::as_str) != Some(WEEK_SCHEMA) {
		problems.push((String::from("schema"), "must be \"some-ui.rust-week/1\""));
	}
	let id = week.get("id").and_then(Value::as_str).filter(|id| is_week_id(id));
	if id.is_none() {
		problems.push((String::from("id"), "must be an ISO year and week, like 2026-w46"));
	}
	let mut programs = Vec::new();
	match week.get("tickets").and_then(Value::as_array) {
		Some(tickets) if !tickets.is_empty() => {
			for (index, ticket) in tickets.iter().enumerate() {
				let at = |field: &str| String::from("tickets[") + &index.to_string() + "]" + field;
				let key = ticket.get("id").and_then(Value::as_str).filter(|key| is_ticket_key(key));
				let Some(key) = key else {
					problems.push((at(".id"), "must be a key like QM-311"));
					continue;
				};
				if programs.iter().any(|held: &TicketProgram| held.ticket == key) {
					problems.push((at(".id"), "is used by an earlier ticket"));
					continue;
				}
				let Some(source) = ticket.get("program").and_then(Value::as_str) else {
					problems.push((at(".program"), "must be the program, as text"));
					continue;
				};
				programs.push(TicketProgram {
					ticket: key.to_owned(),
					source: source.to_owned(),
				});
			}
		}
		_ => problems.push((String::from("tickets"), "must hold at least one ticket")),
	}
	match id {
		Some(id) if problems.is_empty() => Ok(Week { id: id.to_owned(), programs }),
		_ => Err(problems),
	}
}

/// `YYYY-wNN`, the week 01 to 53.
#[must_use]
pub fn is_week_id(id: &str) -> bool {
	let Some((year, week)) = id.split_once("-w") else {
		return false;
	};
	year.len() == 4 && year.bytes().all(|byte| byte.is_ascii_digit()) && week.len() == 2 && week.parse::<u8>().is_ok_and(|week| (1..=53).contains(&week))
}

/// A ticket's key, as the format writes one: `QM-311`.
///
/// A capital, up to five more capitals or digits, a hyphen, and one to six
/// digits. Never a slug, so a note's ref naming a ticket can't be read as one
/// naming a card.
#[must_use]
pub fn is_ticket_key(key: &str) -> bool {
	let Some((project, number)) = key.split_once('-') else {
		return false;
	};
	let mut project = project.bytes();
	project.next().is_some_and(|first| first.is_ascii_uppercase())
		&& project.len() <= 5
		&& project.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
		&& (1..=6).contains(&number.len())
		&& number.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
	use super::{is_ticket_key, is_week_id, mailbox_name, read, TicketProgram};
	use serde_json::json;

	fn week(tickets: &serde_json::Value) -> Vec<u8> {
		json!({ "schema": "some-ui.rust-week/1", "id": "2026-w46", "company": { "name": "Quillmoss" }, "tickets": tickets })
			.to_string()
			.into_bytes()
	}

	#[test]
	fn a_week_gives_each_tickets_program_in_order() {
		let body = week(&json!([
			{ "id": "QM-311", "program": "fn main() {}\n", "speakTo": ["a", "b"] },
			{ "id": "QM-312", "program": "fn main() { x }\n" },
		]));
		let read = read(&body).unwrap();
		assert_eq!(read.id, "2026-w46");
		assert_eq!(
			read.programs,
			vec![
				TicketProgram {
					ticket: String::from("QM-311"),
					source: String::from("fn main() {}\n")
				},
				TicketProgram {
					ticket: String::from("QM-312"),
					source: String::from("fn main() { x }\n")
				},
			]
		);
		assert_eq!(mailbox_name(&read.id), "rust-week-2026-w46.json");
	}

	#[test]
	fn a_week_home_cannot_record_says_every_problem() {
		let body = json!({ "schema": "some-ui.rust-round/2", "id": "week-46", "tickets": [
			{ "id": "qm-311", "program": "fn main() {}\n" },
			{ "id": "QM-312" },
			{ "id": "QM-313", "program": "a" },
			{ "id": "QM-313", "program": "b" },
		] })
		.to_string();
		let fields: Vec<String> = read(body.as_bytes()).unwrap_err().into_iter().map(|(field, _)| field).collect();
		assert_eq!(fields, ["schema", "id", "tickets[0].id", "tickets[1].program", "tickets[3].id"]);
	}

	#[test]
	fn a_week_with_no_tickets_or_too_large_is_refused() {
		assert_eq!(read(&week(&json!([]))).unwrap_err()[0].0, "tickets");
		assert_eq!(read(&vec![b' '; super::WEEK_CEILING + 1]).unwrap_err()[0].0, "body");
		assert_eq!(read(b"[1]").unwrap_err()[0].0, "body");
	}

	#[test]
	fn ids_and_keys() {
		assert!(is_week_id("2026-w46") && is_week_id("2026-w01") && is_week_id("2026-w53"));
		assert!(!is_week_id("2026-w00") && !is_week_id("2026-w54") && !is_week_id("26-w46") && !is_week_id("2026-46"));
		assert!(is_ticket_key("QM-311") && is_ticket_key("Q2RUST-1") && is_ticket_key("A-123456"));
		assert!(!is_ticket_key("qm-311") && !is_ticket_key("QM311") && !is_ticket_key("QM-") && !is_ticket_key("TOOLONG-1") && !is_ticket_key("QM-1234567"));
	}
}
