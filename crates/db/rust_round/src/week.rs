//! What home reads of a week of Rust Rounds: the least it needs to keep it
//! and to record its programs (paulgsc/some-ui#2687, RR12; `/2`: #3249 RR25,
//! #3257 RR29).
//!
//! The format is paulgsc/some-ui's `some-ui.rust-week/1` and `/2`
//! (`packages/ui/rust-rounds/src/lib/formats/week.ts`, RR11), and the
//! routine holds a week to all of it (`readWeek`) before it puts one in the
//! mailbox, as `rust-week-<id>.json` in `brief` ([`MAILBOX_PREFIX`]). Home
//! checks only:
//!
//! - its size ([`WEEK_CEILING`] for `/1`, [`WEEK_V2_CEILING`] for `/2`), its
//!   encoding and that it is one JSON object;
//! - its `schema` tag, [`WEEK_SCHEMA`] or [`WEEK_SCHEMA_V2`];
//! - its `id`, the ISO week it opens (`2026-w46`), which names it for good;
//! - its tickets' ids and every ticket's `program`, the bytes it records;
//! - how many speak-to points each ticket has (RR24, paulgsc/some-ui#3248),
//!   so a take of one point can be checked against them: `speakTo`'s length
//!   in a `/1` week, `forks`' in a `/2` week, where fork n is point n
//!   ([`points_field`]). The points' words are never read here;
//! - in a `/2` week, each ticket's `academy`: 1 to [`ACADEMY_MAX`]
//!   exercises, and each exercise's `starter`, which it records too, under
//!   [`academy_key`] (`TB-101/academy/1`);
//! - its `opens` and `due`, when they are RFC 3339 instants, for the week's
//!   record (RR14, paulgsc/some-ui#2689). A week without them is still kept
//!   and recorded, and has no record.
//!
//! Anything else (the company, the story, the speak-to points' words or
//! forks, an exercise's prose and `expect`, the tutor's guess) is kept as it
//! came and never read here. A `/1` week reads exactly as it did before `/2`:
//! an `academy` on one of its tickets is not read.

use crate::round::Problem;
use serde_json::Value;

/// The first format's tag.
pub const WEEK_SCHEMA: &str = "some-ui.rust-week/1";

/// The format with forks and an academy (RR25, RR29).
pub const WEEK_SCHEMA_V2: &str = "some-ui.rust-week/2";

/// The most bytes a `/1` week may hold: a round's ceiling.
///
/// A week is two to four tickets, each program at most 25 lines of 40
/// columns, and their prose: a few KiB, so each program fits the runner's
/// request with room.
pub const WEEK_CEILING: usize = crate::round::ROUND_CEILING;

/// The most bytes a `/2` week may hold: 128 KiB.
///
/// The most a `/2` week can need, as the format bounds it: 4 tickets of 8
/// exercises is 32 starters, each a whole `main.rs` of about 1.5 KB, so
/// 32 × 1.5 KB = 48 KB, about 56 KB once JSON escapes its newlines and
/// quotes (≈ +15%). Each exercise's prose (title, why, tries, clarity,
/// keywords, tests, expect) is about 1 KB more: 32 KB. Each ticket's forks
/// (up to 6 questions of 2 to 6 options, each a decision, its case and the
/// model it rests on), program and spec are about 4 KB: 16 KB. The week's
/// own prose (company, story, SLA, references) is about 2 KB. In all
/// 56 + 32 + 16 + 2 = 106 KB, under 128 KiB with a fifth to spare. The
/// sample `/2` week, Tallowbrook (3 tickets, 5 exercises), is 11.5 KB.
///
/// Each program a week holds still goes to the runner alone, one request
/// each, under the runner's own `REQUEST_CEILING` (64 KiB,
/// `rust_rounds_wire::protocol`), which this does not change: a starter
/// past it is kept, and recorded as `refused`, `too-large`, as any program
/// is. The mailbox takes up to 1 MiB a file (`MAILBOX_BODY_CEILING`).
pub const WEEK_V2_CEILING: usize = 128 * 1024;

/// The most exercises a ticket's academy holds, as the format bounds it.
pub const ACADEMY_MAX: usize = 8;

/// One ticket's program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketProgram {
	/// The ticket's key, like `QM-311`.
	pub ticket: String,
	/// `main.rs`, exactly.
	pub source: String,
	/// How many speak-to points the ticket has; 0 when it lists none, so no
	/// take of a point can name one of its points.
	pub points: usize,
}

/// One exercise's starter, in a `/2` week's academy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Starter {
	/// Its key, [`academy_key`]: `TB-101/academy/1`. Never a ticket's key.
	pub key: String,
	/// The ticket whose academy holds it, `TB-101`.
	pub ticket: String,
	/// Its place in that academy, from 1.
	pub exercise: usize,
	/// `main.rs`, exactly.
	pub source: String,
}

/// A week home can keep: its id, its tickets' programs in ticket order, and
/// (for `/2`) its academies' starters, ticket by ticket in exercise order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Week {
	pub id: String,
	pub programs: Vec<TicketProgram>,
	/// Empty for a `/1` week.
	pub academy: Vec<Starter>,
	/// When the week opens and its deadline, when both are RFC 3339
	/// instants and the deadline comes after the opening; `None` otherwise.
	pub window: Option<Window>,
}

/// When a week opens and when its deadline passes, as the week writes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
	/// Monday 00:00, the owner's time.
	pub opens: String,
	/// Sunday 23:59, the owner's time. A take not sent by then doesn't count.
	pub due: String,
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

/// The key an academy's exercise is recorded under: `<ticket>/academy/<n>`,
/// `n` from 1, as `TB-101/academy/1`.
///
/// It holds a `/`, which no ticket's key can ([`is_ticket_key`]), so a
/// starter's key never names a ticket.
#[must_use]
pub fn academy_key(ticket: &str, exercise: usize) -> String {
	String::from(ticket) + "/academy/" + &exercise.to_string()
}

/// Which format a week says it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Schema {
	V1,
	V2,
}

/// The array whose length is a ticket's speak-to points: `speakTo` in a
/// `/1` week, `forks` in a `/2` week (RR29 replaced the one with the other,
/// and the phone's `upgradeWeek` turns `speakTo[i]` into `forks[i]`, so a
/// take's point means the same in both).
fn points_field(schema: Option<Schema>) -> &'static str {
	if schema == Some(Schema::V2) {
		"forks"
	} else {
		"speakTo"
	}
}

/// Read `body` as a week, or say every problem found.
///
/// # Errors
/// Each [`Problem`]: a body past [`WEEK_V2_CEILING`], or a `/1` (or
/// unknown) week past [`WEEK_CEILING`]; not UTF-8 or not one JSON object; a
/// missing or other `schema`; an `id` that is not an ISO week; no
/// `tickets`; a ticket whose id is not a key or is used twice, or that has
/// no `program`; in a `/2` week, a ticket whose `academy` does not hold 1 to
/// [`ACADEMY_MAX`] exercises, or an exercise with no `starter`.
pub fn read(body: &[u8]) -> Result<Week, Vec<Problem>> {
	let problem = |field: &str, why| vec![(field.to_owned(), why)];
	if body.len() > WEEK_V2_CEILING {
		return Err(problem("body", "is larger than 128 KiB"));
	}
	let Ok(text) = std::str::from_utf8(body) else {
		return Err(problem("body", "is not UTF-8"));
	};
	let Ok(Value::Object(week)) = serde_json::from_str::<Value>(text) else {
		return Err(problem("body", "is not one JSON object"));
	};
	let schema = match week.get("schema").and_then(Value::as_str) {
		Some(WEEK_SCHEMA) => Some(Schema::V1),
		Some(WEEK_SCHEMA_V2) => Some(Schema::V2),
		_ => None,
	};
	// Only a `/2` week may be larger than a `/1`'s ceiling.
	if schema != Some(Schema::V2) && body.len() > WEEK_CEILING {
		return Err(problem("body", "is larger than 32 KiB"));
	}
	let mut problems = Vec::new();
	if schema.is_none() {
		problems.push((String::from("schema"), "must be \"some-ui.rust-week/1\" or \"some-ui.rust-week/2\""));
	}
	let id = week.get("id").and_then(Value::as_str).filter(|id| is_week_id(id));
	if id.is_none() {
		problems.push((String::from("id"), "must be an ISO year and week, like 2026-w46"));
	}
	let mut programs = Vec::new();
	let mut academy = Vec::new();
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
					points: ticket.get(points_field(schema)).and_then(Value::as_array).map_or(0, Vec::len),
				});
				if schema == Some(Schema::V2) {
					read_academy(ticket, key, &at, &mut academy, &mut problems);
				}
			}
		}
		_ => problems.push((String::from("tickets"), "must hold at least one ticket")),
	}
	let instant = |field: &str| {
		week
			.get(field)
			.and_then(Value::as_str)
			.filter(|at| chrono::DateTime::parse_from_rfc3339(at).is_ok())
			.map(str::to_owned)
	};
	let window = match (instant("opens"), instant("due")) {
		(Some(opens), Some(due)) if crate::instant_order(&opens, &due) == Some(std::cmp::Ordering::Less) => Some(Window { opens, due }),
		_ => None,
	};
	match id {
		Some(id) if problems.is_empty() => Ok(Week {
			id: id.to_owned(),
			programs,
			academy,
			window,
		}),
		_ => Err(problems),
	}
}

/// Each starter of `ticket`'s academy (`key`), onto `academy`, or the
/// problems with it onto `problems`.
fn read_academy(ticket: &Value, key: &str, at: &impl Fn(&str) -> String, academy: &mut Vec<Starter>, problems: &mut Vec<Problem>) {
	let exercises = ticket
		.get("academy")
		.and_then(Value::as_array)
		.filter(|exercises| (1..=ACADEMY_MAX).contains(&exercises.len()));
	let Some(exercises) = exercises else {
		problems.push((at(".academy"), "must hold 1 to 8 exercises"));
		return;
	};
	for (index, exercise) in exercises.iter().enumerate() {
		let Some(source) = exercise.get("starter").and_then(Value::as_str) else {
			problems.push((at(&(String::from(".academy[") + &index.to_string() + "].starter")), "must be the starter, as text"));
			continue;
		};
		academy.push(Starter {
			key: academy_key(key, index + 1),
			ticket: key.to_owned(),
			exercise: index + 1,
			source: source.to_owned(),
		});
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
	use super::{academy_key, is_ticket_key, is_week_id, mailbox_name, read, Starter, TicketProgram, WEEK_CEILING, WEEK_V2_CEILING};
	use serde_json::{json, Value};

	/// paulgsc/some-ui's sample `/2` week, Tallowbrook
	/// (`packages/ui/rust-rounds/src/lib/sample-week/tallowbrook.ts`), as JSON.
	const TALLOWBROOK: &str = include_str!("../testdata/rust-week-2026-w47.json");

	fn tallowbrook() -> Value {
		serde_json::from_str(TALLOWBROOK).unwrap()
	}

	/// `week` grown to `size` + 1 bytes by its company's prose, which home
	/// never reads.
	fn grown(mut week: Value, size: usize) -> Vec<u8> {
		week["company"]["about"] = json!("");
		let short = week.to_string().len();
		week["company"]["about"] = json!("x".repeat(size + 1 - short));
		let body = week.to_string().into_bytes();
		assert_eq!(body.len(), size + 1);
		body
	}

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
					source: String::from("fn main() {}\n"),
					points: 2,
				},
				TicketProgram {
					ticket: String::from("QM-312"),
					source: String::from("fn main() { x }\n"),
					points: 0,
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

	/// A `/2` week gives each ticket's program, as `/1` does, and each
	/// exercise's starter under its key, ticket by ticket in exercise order.
	#[test]
	fn a_v2_week_gives_each_tickets_program_and_its_academys_starters() {
		let file = tallowbrook();
		let read = read(TALLOWBROOK.as_bytes()).unwrap();
		assert_eq!(read.id, "2026-w47");
		let tickets: Vec<&str> = read.programs.iter().map(|program| program.ticket.as_str()).collect();
		assert_eq!(tickets, ["TB-101", "TB-102", "TB-103"]);
		assert_eq!(read.programs[0].source, file["tickets"][0]["program"].as_str().unwrap());
		let keys: Vec<&str> = read.academy.iter().map(|starter| starter.key.as_str()).collect();
		assert_eq!(keys, ["TB-101/academy/1", "TB-101/academy/2", "TB-101/academy/3", "TB-102/academy/1", "TB-103/academy/1"]);
		assert_eq!(
			read.academy[2],
			Starter {
				key: String::from("TB-101/academy/3"),
				ticket: String::from("TB-101"),
				exercise: 3,
				source: file["tickets"][0]["academy"][2]["starter"].as_str().unwrap().to_owned(),
			}
		);
		assert!(read.window.is_some());
	}

	/// A `/1` week reads as it always did: an academy on one of its tickets
	/// is not read, and it holds no starters.
	#[test]
	fn a_v1_week_reads_no_academy() {
		let body = week(&json!([{ "id": "QM-311", "program": "fn main() {}\n", "academy": [{ "starter": "fn main() {}\n" }] }]));
		assert!(read(&body).unwrap().academy.is_empty());
		let body = week(&json!([{ "id": "QM-311", "program": "fn main() {}\n", "academy": "anything" }]));
		assert!(read(&body).is_ok());
	}

	/// A `/2` ticket's academy holds 1 to 8 exercises, each with a starter.
	#[test]
	fn a_v2_week_home_cannot_record_says_every_problem() {
		let mut file = tallowbrook();
		file["tickets"][0]["academy"] = json!([]);
		file["tickets"][1]["academy"][0].as_object_mut().unwrap().remove("starter");
		file["tickets"][2]["academy"] = json!(vec![json!({ "starter": "fn main() {}\n" }); 9]);
		let fields: Vec<String> = read(file.to_string().as_bytes()).unwrap_err().into_iter().map(|(field, _)| field).collect();
		assert_eq!(fields, ["tickets[0].academy", "tickets[1].academy[0].starter", "tickets[2].academy"]);

		let mut file = tallowbrook();
		file["tickets"][0].as_object_mut().unwrap().remove("academy");
		file["tickets"][1]["academy"] = json!(vec![json!({ "starter": "fn main() {}\n" }); 8]);
		let fields: Vec<String> = read(file.to_string().as_bytes()).unwrap_err().into_iter().map(|(field, _)| field).collect();
		assert_eq!(fields, ["tickets[0].academy"], "eight is the most, and allowed");
	}

	/// A `/2` ticket's points are its forks (fork n is point n); a `speakTo`
	/// it does not carry is not counted.
	#[test]
	fn a_v2_tickets_points_are_its_forks() {
		let read_week = read(TALLOWBROOK.as_bytes()).unwrap();
		let points: Vec<(&str, usize)> = read_week.programs.iter().map(|program| (program.ticket.as_str(), program.points)).collect();
		assert_eq!(points, [("TB-101", 2), ("TB-102", 1), ("TB-103", 1)]);

		let mut file = tallowbrook();
		file["tickets"][0]["forks"] = json!(vec![json!({ "question": "q" }); 6]);
		file["tickets"][1]["speakTo"] = json!(["a", "b", "c"]);
		let read_week = read(file.to_string().as_bytes()).unwrap();
		assert_eq!(read_week.programs[0].points, 6, "six forks, six points");
		assert_eq!(read_week.programs[1].points, 1, "a /2 ticket's speakTo is not read");
	}

	/// A `/2` week may be larger than a `/1` week's ceiling, up to its own;
	/// a `/1` week, or one of no known schema, may not.
	#[test]
	fn a_v2_week_has_its_own_ceiling() {
		let v2 = grown(tallowbrook(), WEEK_CEILING);
		assert!(read(&v2).is_ok(), "a /2 week past 32 KiB and under 128 KiB is kept");
		let at_most = grown(tallowbrook(), WEEK_V2_CEILING - 1);
		assert_eq!(at_most.len(), WEEK_V2_CEILING);
		assert!(read(&at_most).is_ok(), "128 KiB exactly is kept");
		assert_eq!(
			read(&grown(tallowbrook(), WEEK_V2_CEILING)).unwrap_err(),
			[(String::from("body"), "is larger than 128 KiB")]
		);

		let mut v1 = tallowbrook();
		v1["schema"] = json!("some-ui.rust-week/1");
		assert_eq!(read(&grown(v1, WEEK_CEILING)).unwrap_err(), [(String::from("body"), "is larger than 32 KiB")]);
		let mut unknown = tallowbrook();
		unknown["schema"] = json!("some-ui.rust-week/3");
		assert_eq!(read(&grown(unknown, WEEK_CEILING)).unwrap_err()[0].0, "body");
	}

	/// No starter's key is ever a ticket's, whatever ticket or exercise.
	#[test]
	fn a_starter_key_is_never_a_ticket_key() {
		for ticket in ["TB-101", "Q2RUST-1", "A-123456", "QM-311"] {
			assert!(is_ticket_key(ticket));
			for exercise in 1..=super::ACADEMY_MAX {
				let key = academy_key(ticket, exercise);
				assert!(!is_ticket_key(&key), "{key}");
				assert!(key.starts_with(ticket) && key.contains('/'), "{key}");
			}
		}
		assert_eq!(academy_key("TB-101", 1), "TB-101/academy/1");
	}

	#[test]
	fn ids_and_keys() {
		assert!(is_week_id("2026-w46") && is_week_id("2026-w01") && is_week_id("2026-w53"));
		assert!(!is_week_id("2026-w00") && !is_week_id("2026-w54") && !is_week_id("26-w46") && !is_week_id("2026-46"));
		assert!(is_ticket_key("QM-311") && is_ticket_key("Q2RUST-1") && is_ticket_key("A-123456"));
		assert!(!is_ticket_key("qm-311") && !is_ticket_key("QM311") && !is_ticket_key("QM-") && !is_ticket_key("TOOLONG-1") && !is_ticket_key("QM-1234567"));
	}
}
