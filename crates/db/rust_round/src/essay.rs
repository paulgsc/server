//! A week's Rust Rounds essay, as home renders it (RR16,
//! paulgsc/some-ui#2691).
//!
//! The essay (`some-ui.rust-essay/1`, paulgsc/some-ui
//! `packages/video-render/src/essay.ts`) is the routine's script for the
//! video that answers a week after its deadline. It rides the mailbox's
//! `brief` folder as `rust-essay-<week>.json`, as the week itself does
//! (`crate::week::MAILBOX_PREFIX`; owner, paulgsc/some-ui#2684, 2026-11-07),
//! and is kept as any `brief` file is. It never goes to `paulgsc/testing`.
//!
//! **Home reads only what it must check.** The recorder parses the whole
//! essay (`parseEssay`) and refuses one that contradicts itself; home reads
//! its schema, its week and each ticket's answer kind ([`read`]), and checks
//! those against what the owner sent of the week by its deadline ([`against`]):
//! an essay may play only takes the week's final record names, and must
//! play every one of them. Nothing the essay says is read past that, and
//! nothing of it reaches a log line.

use crate::round::Problem;
use crate::sent::Sent;
use crate::week::{is_ticket_key, is_week_id};
use serde_json::Value;

/// The essay's format, as `essay.ts` writes it (`ESSAY_SCHEMA`).
pub const ESSAY_SCHEMA: &str = "some-ui.rust-essay/1";

/// What an essay's name in the mailbox starts with.
pub const MAILBOX_PREFIX: &str = "rust-essay-";

/// The most bytes an essay may hold: four tickets, each with its program,
/// transcript, best fit and two deep dives, are tens of KiB.
pub const ESSAY_CEILING: usize = 512 * 1024;

/// The mailbox name an essay is put under: `rust-essay-<week>.json`.
#[must_use]
pub fn mailbox_name(week: &str) -> String {
	String::from(MAILBOX_PREFIX) + week + ".json"
}

/// How a ticket of the essay is answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answered {
	/// The owner's take, played whole in its answer scene.
	Take,
	/// Nothing sent by the deadline: the silence, and a guess at why.
	Silence,
}

/// What home reads of an essay: its week and each ticket's answer, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Essay {
	pub week: String,
	pub tickets: Vec<(String, Answered)>,
}

/// Read `body` as an essay, as far as home checks one, or say every problem.
///
/// # Errors
/// Each [`Problem`]: a body past [`ESSAY_CEILING`], not UTF-8 or not one
/// JSON object; another `schema`; a `week.id` that is not an ISO week; no
/// tickets; a ticket whose id is not a key or is used twice, or whose
/// `answer.kind` is neither `take` nor `silence`.
pub fn read(body: &[u8]) -> Result<Essay, Vec<Problem>> {
	let problem = |field: &str, why| vec![(field.to_owned(), why)];
	if body.len() > ESSAY_CEILING {
		return Err(problem("body", "is larger than 512 KiB"));
	}
	let Ok(text) = std::str::from_utf8(body) else {
		return Err(problem("body", "is not UTF-8"));
	};
	let Ok(Value::Object(essay)) = serde_json::from_str::<Value>(text) else {
		return Err(problem("body", "is not one JSON object"));
	};
	let mut problems = Vec::new();
	if essay.get("schema").and_then(Value::as_str) != Some(ESSAY_SCHEMA) {
		problems.push((String::from("schema"), "must be \"some-ui.rust-essay/1\""));
	}
	let week = essay.get("week").and_then(|week| week.get("id")).and_then(Value::as_str).filter(|id| is_week_id(id));
	if week.is_none() {
		problems.push((String::from("week.id"), "must be an ISO year and week, like 2026-w46"));
	}
	let mut tickets: Vec<(String, Answered)> = Vec::new();
	match essay.get("tickets").and_then(Value::as_array) {
		Some(held) if !held.is_empty() => {
			for (index, ticket) in held.iter().enumerate() {
				let at = |field: &str| String::from("tickets[") + &index.to_string() + "]" + field;
				let Some(key) = ticket.get("id").and_then(Value::as_str).filter(|key| is_ticket_key(key)) else {
					problems.push((at(".id"), "must be a key like QM-311"));
					continue;
				};
				if tickets.iter().any(|(held, _)| held == key) {
					problems.push((at(".id"), "is used by an earlier ticket"));
					continue;
				}
				let answered = match ticket.get("answer").and_then(|answer| answer.get("kind")).and_then(Value::as_str) {
					Some("take") => Answered::Take,
					Some("silence") => Answered::Silence,
					_ => {
						problems.push((at(".answer.kind"), "must be \"take\" or \"silence\""));
						continue;
					}
				};
				tickets.push((key.to_owned(), answered));
			}
		}
		_ => problems.push((String::from("tickets"), "must hold at least one ticket")),
	}
	match week {
		Some(week) if problems.is_empty() => Ok(Essay { week: week.to_owned(), tickets }),
		_ => Err(problems),
	}
}

/// One take the essay plays: its ticket and the voice note that answers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Played {
	pub ticket: String,
	/// The take's voice-note id, as the week's final record names it.
	pub take: String,
}

/// Check `essay` against what the owner sent of its week by the deadline:
/// `sent` is the week's final, or `None` when nothing was sent. Each take the
/// essay plays, in the essay's order.
///
/// An essay may play only the takes the final names, and must play each one:
/// a ticket the final answers is a `take`, every other ticket a `silence`,
/// and every ticket the final answers is one of the essay's.
///
/// # Errors
/// Each [`Problem`], naming the ticket by its place in the essay or, for one
/// the essay lacks, by its key.
pub fn against(essay: &Essay, sent: Option<&Sent>) -> Result<Vec<Played>, Vec<Problem>> {
	let answers = sent.map_or(&[][..], |sent| sent.answers.as_slice());
	let mut problems = Vec::new();
	let mut played = Vec::new();
	for (index, (ticket, answered)) in essay.tickets.iter().enumerate() {
		let at = String::from("tickets[") + &index.to_string() + "].answer.kind";
		let take = answers.iter().find(|answer| &answer.ticket == ticket);
		match (answered, take) {
			(Answered::Take, Some(answer)) => played.push(Played {
				ticket: ticket.clone(),
				take: answer.take.clone(),
			}),
			(Answered::Take, None) => problems.push((at, "is a take, and the week's final record sent none for this ticket")),
			(Answered::Silence, Some(_)) => problems.push((at, "is a silence, and the week's final record sent a take for this ticket")),
			(Answered::Silence, None) => {}
		}
	}
	for answer in answers {
		if !essay.tickets.iter().any(|(ticket, _)| ticket == &answer.ticket) {
			problems.push((String::from("tickets[") + &answer.ticket + "]", "was answered by the deadline, and the essay leaves it out"));
		}
	}
	if problems.is_empty() {
		Ok(played)
	} else {
		Err(problems)
	}
}

#[cfg(test)]
mod tests {
	use super::{against, mailbox_name, read, Answered, Essay, Played};
	use crate::sent::{Answer, Sent};
	use serde_json::json;

	fn body(value: &serde_json::Value) -> Vec<u8> {
		value.to_string().into_bytes()
	}

	fn essay() -> serde_json::Value {
		json!({
			"schema": "some-ui.rust-essay/1",
			"week": { "id": "2026-w46", "from": "2026-11-09", "to": "2026-11-15" },
			"tickets": [
				{ "id": "QM-311", "answer": { "kind": "take", "transcript": "every field" } },
				{ "id": "QM-312", "answer": { "kind": "silence", "opened": 2, "guess": "no time" } },
			],
		})
	}

	fn sent(answers: &[(&str, &str)]) -> Sent {
		Sent {
			week: String::from("2026-w46"),
			at: String::from("2026-11-15T20:00:00.000Z"),
			answers: answers
				.iter()
				.map(|(ticket, take)| Answer {
					ticket: (*ticket).to_owned(),
					take: (*take).to_owned(),
					recorded_at: String::from("2026-11-15T19:00:00.000Z"),
				})
				.collect(),
			passed: Vec::new(),
		}
	}

	#[test]
	fn an_essay_is_named_and_read_for_its_week_and_each_tickets_answer() {
		assert_eq!(mailbox_name("2026-w46"), "rust-essay-2026-w46.json");
		assert_eq!(
			read(&body(&essay())).unwrap(),
			Essay {
				week: String::from("2026-w46"),
				tickets: vec![(String::from("QM-311"), Answered::Take), (String::from("QM-312"), Answered::Silence)],
			}
		);
		let mut wrong = essay();
		wrong["schema"] = json!("some-ui.rust-week/1");
		wrong["week"]["id"] = json!("2026-46");
		wrong["tickets"][1]["id"] = json!("QM-311");
		wrong["tickets"].as_array_mut().unwrap().push(json!({ "id": "QM-313", "answer": { "kind": "maybe" } }));
		let fields: Vec<_> = read(&body(&wrong)).unwrap_err().into_iter().map(|(field, _)| field).collect();
		assert_eq!(fields, ["schema", "week.id", "tickets[1].id", "tickets[2].answer.kind"]);
		assert!(read(b"[]").is_err());
		assert!(read(&vec![b' '; super::ESSAY_CEILING + 1]).is_err());
	}

	#[test]
	fn an_essay_plays_exactly_the_takes_the_weeks_final_record_names() {
		let essay = read(&body(&essay())).unwrap();
		assert_eq!(
			against(&essay, Some(&sent(&[("QM-311", "note-1")]))).unwrap(),
			[Played {
				ticket: String::from("QM-311"),
				take: String::from("note-1"),
			}]
		);
		let fields = |sent: Option<&Sent>| -> Vec<String> { against(&essay, sent).unwrap_err().into_iter().map(|(field, _)| field).collect() };
		assert_eq!(fields(None), ["tickets[0].answer.kind"], "nothing sent: a take it plays is none of the owner's");
		assert_eq!(
			fields(Some(&sent(&[("QM-311", "n1"), ("QM-312", "n2")]))),
			["tickets[1].answer.kind"],
			"a take called a silence"
		);
		assert_eq!(
			fields(Some(&sent(&[("QM-311", "n1"), ("QM-313", "n3")]))),
			["tickets[QM-313]"],
			"a ticket answered and left out"
		);
	}
}
