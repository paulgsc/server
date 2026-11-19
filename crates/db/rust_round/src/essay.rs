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

use crate::draft::Kind;
use crate::round::{is_slug, Problem};
use crate::week::{is_ticket_key, is_week_id};
use serde_json::Value;

/// The essay's format, as `essay.ts` writes it (`ESSAY_SCHEMA`).
pub const ESSAY_SCHEMA: &str = "some-ui.rust-essay/1";

/// What an essay's name in the mailbox starts with.
pub const MAILBOX_PREFIX: &str = "rust-essay-";

/// A week of mock interview's essay's format (IW6, paulgsc/some-ui#2937).
pub const INTERVIEW_ESSAY_SCHEMA: &str = "some-ui.interview-essay/1";

/// What a week of mock interview's essay's name in the mailbox starts with.
pub const INTERVIEW_MAILBOX_PREFIX: &str = "interview-essay-";

/// The most bytes an essay may hold: four tickets, each with its program,
/// transcript, best fit and two deep dives, are tens of KiB.
pub const ESSAY_CEILING: usize = 512 * 1024;

/// One list of an essay's items, as home reads it: the field that holds it,
/// what an item's id must be, the words a [`Problem`] about one says, and the
/// prefix the take's file is named with in the render's `takes/`.
struct List {
	field: &'static str,
	is_id: fn(&str) -> bool,
	id_why: &'static str,
	id_used: &'static str,
	take_none: &'static str,
	silence_sent: &'static str,
	file_prefix: &'static str,
}

/// What differs between kinds of essay, as home reads one: its schema and
/// name, and the lists of items it plays.
///
/// A Rust Rounds essay plays a week's tickets (`tickets[]`, each keyed
/// `QM-311`); a week of mock interview's plays its questions (`questions[]`,
/// each a slug, as the week names them, `crate::interview`) and its
/// teleprompter cards read (`reads[]`, each a card's slug). Everything else
/// is one rule for every kind and every list.
struct Words {
	schema: &'static str,
	schema_why: &'static str,
	prefix: &'static str,
	week_why: &'static str,
	/// The list every essay has at least one item of.
	items: List,
	no_items: &'static str,
	/// A second list, if the kind has one: none or any number of items, and
	/// a list when it is there.
	more: Option<List>,
	more_why: &'static str,
}

const fn words(kind: Kind) -> &'static Words {
	match kind {
		Kind::RustRounds => &Words {
			schema: ESSAY_SCHEMA,
			schema_why: "must be \"some-ui.rust-essay/1\"",
			prefix: MAILBOX_PREFIX,
			week_why: "must be an ISO year and week, like 2026-w46",
			items: List {
				field: "tickets",
				is_id: is_ticket_key,
				id_why: "must be a key like QM-311",
				id_used: "is used by an earlier ticket",
				take_none: "is a take, and the week's final record sent none for this ticket",
				silence_sent: "is a silence, and the week's final record sent a take for this ticket",
				file_prefix: "",
			},
			no_items: "must hold at least one ticket",
			more: None,
			more_why: "",
		},
		Kind::Interview => &Words {
			schema: INTERVIEW_ESSAY_SCHEMA,
			schema_why: "must be \"some-ui.interview-essay/1\"",
			prefix: INTERVIEW_MAILBOX_PREFIX,
			week_why: "must be an ISO year and week, like 2026-w47",
			items: List {
				field: "questions",
				is_id: is_slug,
				id_why: "must be a slug like about-you, at most 64 characters",
				id_used: "is used by an earlier question",
				take_none: "is a take, and the week's final record sent none for this question",
				silence_sent: "is a silence, and the week's final record sent a take for this question",
				file_prefix: "",
			},
			no_items: "must hold at least one question",
			more: Some(List {
				field: "reads",
				is_id: is_slug,
				id_why: "must be a slug like intro, at most 64 characters",
				id_used: "is used by an earlier read",
				take_none: "is a take, and the week's final record read none of this card",
				silence_sent: "is a silence, and the week's final record read this card",
				// A question is never named `read` (`crate::interview`), so
				// `read.<card>` is never a question's file.
				file_prefix: "read.",
			}),
			more_why: "must be a list of the cards read",
		},
	}
}

/// What a `kind` of essay's name in the mailbox starts with.
#[must_use]
pub const fn prefix(kind: Kind) -> &'static str {
	words(kind).prefix
}

/// The mailbox name an essay is put under: `rust-essay-<week>.json`.
#[must_use]
pub fn mailbox_name(week: &str) -> String {
	String::from(MAILBOX_PREFIX) + week + ".json"
}

/// The week and revision a name in the mailbox gives a Rust Rounds essay
/// ([`parse_name_of`]).
#[must_use]
pub fn parse_name(name: &str) -> Option<(String, u32)> {
	parse_name_of(Kind::RustRounds, name)
}

/// The week and revision a name in the mailbox gives a `kind` of essay.
///
/// `rust-essay-<week>.json` is revision 1, a correction
/// `rust-essay-<week>-r<N>.json` revision N (2 or more, no leading zero; the
/// coordinator's C72, paulgsc/some-ui `docs/calls.json`), and likewise
/// `interview-essay-…`. `None` for any other name.
#[must_use]
pub fn parse_name_of(kind: Kind, name: &str) -> Option<(String, u32)> {
	let stem = name.strip_prefix(prefix(kind))?.strip_suffix(".json")?;
	let (week, revision) = match stem.split_once("-r") {
		Some((week, n)) => {
			let first = n.bytes().next()?;
			if first == b'0' || !n.bytes().all(|b| b.is_ascii_digit()) || n.len() > 6 {
				return None;
			}
			let n: u32 = n.parse().ok()?;
			if n < 2 {
				return None;
			}
			(week, n)
		}
		None => (stem, 1),
	};
	is_week_id(week).then(|| (week.to_owned(), revision))
}

/// How an item (a ticket, a question) of the essay is answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answered {
	/// The owner's take, played whole in its answer scene.
	Take,
	/// Nothing sent by the deadline: the silence, and a guess at why.
	Silence,
}

/// What home reads of an essay: its kind, its week and each item's answer.
///
/// In order. An item is a ticket of a Rust Rounds week, a question of a
/// week of mock interview; `reads` are a week of mock interview's cards read,
/// none for a Rust Rounds essay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Essay {
	pub kind: Kind,
	pub week: String,
	pub tickets: Vec<(String, Answered)>,
	/// Each speak-to point a Rust Rounds essay answers on its own (RR24),
	/// as [`PointAnswer`]; none for a mock interview's.
	pub points: Vec<PointAnswer>,
	pub reads: Vec<(String, Answered)>,
}

/// A speak-to point an essay answers on its own (RR24,
/// paulgsc/some-ui#3248): its item, [`point_key`], its answer, and where it
/// sits, `tickets[ticket].points[point]`, which is the field a refusal names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointAnswer {
	pub item: String,
	pub answered: Answered,
	pub ticket: usize,
	pub point: usize,
}

/// Read `body` as a Rust Rounds essay ([`read_of`]).
///
/// # Errors
/// As [`read_of`].
pub fn read(body: &[u8]) -> Result<Essay, Vec<Problem>> {
	read_of(Kind::RustRounds, body)
}

/// Read `body` as a `kind` of essay, as far as home checks one, or say every
/// problem.
///
/// # Errors
/// Each [`Problem`]: a body past [`ESSAY_CEILING`], not UTF-8 or not one
/// JSON object; another `schema`; a `week.id` that is not an ISO week; no
/// items; an item whose id is not the kind's (a ticket's key, a question's
/// slug) or is used twice, or whose `answer.kind` is neither `take` nor
/// `silence`.
pub fn read_of(kind: Kind, body: &[u8]) -> Result<Essay, Vec<Problem>> {
	let words = words(kind);
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
	if essay.get("schema").and_then(Value::as_str) != Some(words.schema) {
		problems.push((String::from("schema"), words.schema_why));
	}
	let week = essay.get("week").and_then(|week| week.get("id")).and_then(Value::as_str).filter(|id| is_week_id(id));
	if week.is_none() {
		problems.push((String::from("week.id"), words.week_why));
	}
	let (tickets, points) = match essay.get(words.items.field).and_then(Value::as_array) {
		Some(held) if !held.is_empty() => {
			let tickets = items(held, &words.items, &mut problems);
			let points = if kind == Kind::RustRounds { point_items(held, &mut problems) } else { Vec::new() };
			(tickets, points)
		}
		_ => {
			problems.push((String::from(words.items.field), words.no_items));
			(Vec::new(), Vec::new())
		}
	};
	let reads = match words.more.as_ref().map(|more| (more, essay.get(more.field))) {
		Some((more, Some(Value::Array(held)))) => items(held, more, &mut problems),
		Some((more, Some(_))) => {
			problems.push((String::from(more.field), words.more_why));
			Vec::new()
		}
		Some((_, None)) | None => Vec::new(),
	};
	match week {
		Some(week) if problems.is_empty() => Ok(Essay {
			kind,
			week: week.to_owned(),
			tickets,
			points,
			reads,
		}),
		_ => Err(problems),
	}
}

/// Each item of `held`, a `list` of an essay, with each problem found.
fn items(held: &[Value], list: &List, problems: &mut Vec<Problem>) -> Vec<(String, Answered)> {
	let mut items: Vec<(String, Answered)> = Vec::new();
	for (index, item) in held.iter().enumerate() {
		let at = |field: &str| String::from(list.field) + "[" + &index.to_string() + "]" + field;
		let Some(key) = item.get("id").and_then(Value::as_str).filter(|key| (list.is_id)(key)) else {
			problems.push((at(".id"), list.id_why));
			continue;
		};
		if items.iter().any(|(held, _)| held == key) {
			problems.push((at(".id"), list.id_used));
			continue;
		}
		let answered = match item.get("answer").and_then(|answer| answer.get("kind")).and_then(Value::as_str) {
			Some("take") => Answered::Take,
			Some("silence") => Answered::Silence,
			_ => {
				problems.push((at(".answer.kind"), "must be \"take\" or \"silence\""));
				continue;
			}
		};
		items.push((key.to_owned(), answered));
	}
	items
}

/// Each speak-to point of a Rust Rounds essay's tickets that answers on its
/// own (RR24, paulgsc/some-ui#3248): `tickets[i].points[j].answer.kind`, a
/// `take` or a `silence`, as the item `<ticket>.<j+1>`. A point with no
/// `answer` is judged within its ticket's, as before RR24.
///
/// The point is the one the take's `ref` names (`<week>:<ticket>:<n>`), as
/// the week's final keeps it ([`rust_final`]): never one split from a take
/// of the whole ticket.
fn point_items(held: &[Value], problems: &mut Vec<Problem>) -> Vec<PointAnswer> {
	let mut items = Vec::new();
	for (index, ticket) in held.iter().enumerate() {
		let Some(key) = ticket.get("id").and_then(Value::as_str).filter(|key| is_ticket_key(key)) else {
			continue;
		};
		let points = ticket.get("points").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
		for (j, point) in points.iter().enumerate() {
			let Some(answer) = point.get("answer") else {
				continue;
			};
			let answered = match answer.get("kind").and_then(Value::as_str) {
				Some("take") => Answered::Take,
				Some("silence") => Answered::Silence,
				_ => {
					let at = String::from("tickets[") + &index.to_string() + "].points[" + &j.to_string() + "].answer.kind";
					problems.push((at, "must be \"take\" or \"silence\""));
					continue;
				}
			};
			items.push(PointAnswer {
				item: point_key(key, j + 1),
				answered,
				ticket: index,
				point: j,
			});
		}
	}
	items
}

/// A point's item and its take's name in the render's `takes/`, less its
/// extension: `QM-311.2`.
#[must_use]
pub fn point_key(ticket: &str, point: usize) -> String {
	String::from(ticket) + "." + &point.to_string()
}

/// One take the essay plays: its item (a ticket, a question, a card read)
/// and the voice note that answers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Played {
	/// The take's name in the render's `takes/`, less its extension: the
	/// ticket's or question's id, or `read.<card>` for a card read.
	pub ticket: String,
	/// The take's voice-note id, as the week's final record names it.
	pub take: String,
}

/// What the owner sent of a week by its deadline, as an essay is checked.
///
/// Its final's answers (each ticket or question with its take) and, for a
/// week of mock interview, its cards read (each card with its take). Both
/// empty when nothing was sent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Final<'a> {
	pub answers: Vec<(&'a str, &'a str)>,
	pub reads: Vec<(&'a str, &'a str)>,
	/// The takes among them an essay may call a silence: home's verdict on
	/// the transcript the routine reads of each is garbled, failed or
	/// nothing said (paulgsc/server#908, HEARD1). An essay may treat one as
	/// unanswered, a silence, or play it.
	pub unheard: Vec<&'a str>,
}

/// Check `essay` against what the owner sent of its week by the deadline.
/// Each take the essay plays, in the essay's order: its items', then its
/// reads'.
///
/// An essay may play only the takes the final names, and must play each one:
/// an item the final answers is a `take`, every other item a `silence`, and
/// every item the final answers is one of the essay's; and likewise each
/// card the final read. An item whose take home could not hear
/// ([`Final::unheard`]) may be a silence too: the routines treat it as
/// unanswered (HEARD2), and the essay then plays nothing for it.
///
/// # Errors
/// Each [`Problem`], naming the item by its place in the essay or, for one
/// the essay lacks, by its id.
pub fn against(essay: &Essay, sent: &Final<'_>) -> Result<Vec<Played>, Vec<Problem>> {
	let words = words(essay.kind);
	let mut problems = Vec::new();
	let mut played = Vec::new();
	let at_fields = |list: &List, held: &[(String, Answered)]| -> Vec<(String, Answered, String)> {
		held
			.iter()
			.enumerate()
			.map(|(index, (item, answered))| (item.clone(), *answered, String::from(list.field) + "[" + &index.to_string() + "].answer.kind"))
			.collect()
	};
	// A point is named by its own field, never by its place among the items.
	let tickets = at_fields(&words.items, &essay.tickets).into_iter().chain(essay.points.iter().map(|point| {
		let at = String::from(words.items.field) + "[" + &point.ticket.to_string() + "].points[" + &point.point.to_string() + "].answer.kind";
		(point.item.clone(), point.answered, at)
	}));
	check(&tickets.collect::<Vec<_>>(), &sent.answers, &sent.unheard, &words.items, &mut played, &mut problems);
	if let Some(more) = &words.more {
		check(&at_fields(more, &essay.reads), &sent.reads, &sent.unheard, more, &mut played, &mut problems);
	}
	if problems.is_empty() {
		Ok(played)
	} else {
		Err(problems)
	}
}

/// [`against`] for one list: the essay's `items` against what the final sent
/// of them.
fn check(items: &[(String, Answered, String)], sent: &[(&str, &str)], unheard: &[&str], list: &List, played: &mut Vec<Played>, problems: &mut Vec<Problem>) {
	for (ticket, answered, at) in items {
		let at = at.clone();
		let take = sent.iter().find(|(answered, _)| answered == ticket);
		match (answered, take) {
			(Answered::Take, Some((_, take))) => played.push(Played {
				ticket: String::from(list.file_prefix) + ticket,
				take: (*take).to_owned(),
			}),
			(Answered::Take, None) => problems.push((at, list.take_none)),
			(Answered::Silence, Some((_, take))) if unheard.contains(take) => {}
			(Answered::Silence, Some(_)) => problems.push((at, list.silence_sent)),
			(Answered::Silence, None) => {}
		}
	}
	for (answered, _) in sent {
		if !items.iter().any(|(ticket, _, _)| ticket == answered) {
			problems.push((String::from(list.field) + "[" + answered + "]", "was answered by the deadline, and the essay leaves it out"));
		}
	}
}

/// A Rust Rounds week's final answers, each keyed as an essay's item: the
/// ticket for a take of the whole ticket, [`point_key`] for one of a point
/// (RR24). [`Final::answers`] borrows them.
#[must_use]
pub fn rust_answers(sent: Option<&crate::sent::Sent>) -> Vec<(String, String)> {
	sent.map_or_else(Vec::new, |sent| {
		sent
			.answers
			.iter()
			.map(|answer| {
				let key = answer.point.map_or_else(|| answer.ticket.clone(), |point| point_key(&answer.ticket, usize::from(point)));
				(key, answer.take.clone())
			})
			.collect()
	})
}

/// A Rust Rounds week's final, as [`against`] takes it, from its keyed
/// answers ([`rust_answers`]).
#[must_use]
pub fn rust_final(answers: &[(String, String)]) -> Final<'_> {
	Final {
		answers: answers.iter().map(|(item, take)| (item.as_str(), take.as_str())).collect(),
		reads: Vec::new(),
		unheard: Vec::new(),
	}
}

/// A week of mock interview's final, as [`against`] takes it: its questions
/// answered and its cards read.
#[must_use]
pub fn interview_final(sent: Option<&crate::interview::Sent>) -> Final<'_> {
	Final {
		answers: sent.map_or_else(Vec::new, |sent| {
			sent.answers.iter().map(|answer| (answer.question.as_str(), answer.take.as_str())).collect()
		}),
		reads: sent.map_or_else(Vec::new, |sent| sent.reads.iter().map(|read| (read.card.as_str(), read.take.as_str())).collect()),
		unheard: Vec::new(),
	}
}

#[cfg(test)]
mod tests {
	use super::{against, interview_final, mailbox_name, parse_name, parse_name_of, read, read_of, rust_answers, rust_final, Answered, Essay, Final, Played};
	use crate::draft::Kind;
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
					point: None,
					take: (*take).to_owned(),
					recorded_at: String::from("2026-11-15T19:00:00.000Z"),
				})
				.collect(),
			passed: Vec::new(),
		}
	}

	/// RR24 (paulgsc/some-ui#3248): a point answered by a take of its own is an
	/// item of the essay, `<ticket>.<n>`, played as `takes/QM-311.2`, beside
	/// the ticket's own answer; the essay must play every point take the final
	/// names, and only those, as it does a ticket's.
	#[test]
	fn a_point_answered_on_its_own_is_played_as_its_own_take() {
		let essay = json!({
			"schema": "some-ui.rust-essay/1",
			"week": { "id": "2026-w46" },
			"tickets": [
				{ "id": "QM-311", "answer": { "kind": "silence" },
					"points": [{ "to": "why" }, { "to": "what", "answer": { "kind": "take" } }, { "to": "when", "answer": { "kind": "silence" } }] },
				{ "id": "QM-312", "answer": { "kind": "take" }, "points": [{ "to": "a" }, { "to": "b" }] },
			],
		});
		let read_essay = read(&body(&essay)).unwrap();
		let mut week = sent(&[("QM-311", "point-2"), ("QM-312", "whole")]);
		week.answers[0].point = Some(2);
		let answers = rust_answers(Some(&week));
		assert_eq!(answers[0], (String::from("QM-311.2"), String::from("point-2")));
		assert_eq!(
			against(&read_essay, &rust_final(&answers)).unwrap(),
			[
				Played {
					ticket: String::from("QM-312"),
					take: String::from("whole")
				},
				Played {
					ticket: String::from("QM-311.2"),
					take: String::from("point-2")
				},
			]
		);
		// An essay that leaves the point's take out, or calls it a silence, is refused.
		let mut unplayed = essay.clone();
		unplayed["tickets"][0]["points"][1].as_object_mut().unwrap().remove("answer");
		assert_eq!(against(&read(&body(&unplayed)).unwrap(), &rust_final(&answers)).unwrap_err()[0].0, "tickets[QM-311.2]");
		let mut silent = essay.clone();
		silent["tickets"][0]["points"][1]["answer"]["kind"] = json!("silence");
		assert_eq!(
			against(&read(&body(&silent)).unwrap(), &rust_final(&answers)).unwrap_err()[0].0,
			"tickets[0].points[1].answer.kind",
			"a point's refusal names its own field, not its place among the items"
		);
		let mut bad = essay;
		bad["tickets"][0]["points"][1]["answer"]["kind"] = json!("maybe");
		assert_eq!(read(&body(&bad)).unwrap_err()[0].0, "tickets[0].points[1].answer.kind");
	}

	#[test]
	fn an_essay_is_named_and_read_for_its_week_and_each_tickets_answer() {
		assert_eq!(mailbox_name("2026-w46"), "rust-essay-2026-w46.json");
		assert_eq!(parse_name("rust-essay-2026-w46.json"), Some((String::from("2026-w46"), 1)));
		assert_eq!(parse_name("rust-essay-2026-w46-r2.json"), Some((String::from("2026-w46"), 2)));
		assert_eq!(parse_name("rust-essay-2026-w46-r12.json"), Some((String::from("2026-w46"), 12)));
		for not in [
			"rust-essay-2026-w46-r1.json",
			"rust-essay-2026-w46-r02.json",
			"rust-essay-2026-w46-r.json",
			"rust-essay-2026-w46-rx.json",
			"rust-essay-2026-w99.json",
			"rust-week-2026-w46.json",
		] {
			assert_eq!(parse_name(not), None, "{not}");
		}
		assert_eq!(
			read(&body(&essay())).unwrap(),
			Essay {
				kind: Kind::RustRounds,
				week: String::from("2026-w46"),
				tickets: vec![(String::from("QM-311"), Answered::Take), (String::from("QM-312"), Answered::Silence)],
				points: Vec::new(),
				reads: Vec::new(),
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
			against(&essay, &rust_final(&rust_answers(Some(&sent(&[("QM-311", "note-1")]))))).unwrap(),
			[Played {
				ticket: String::from("QM-311"),
				take: String::from("note-1"),
			}]
		);
		let fields = |sent: Option<&Sent>| -> Vec<String> { against(&essay, &rust_final(&rust_answers(sent))).unwrap_err().into_iter().map(|(field, _)| field).collect() };
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

	/// HEARD1: a take home found garbled or failed may be a silence, and
	/// may still be played; a take home heard is a take, as before.
	#[test]
	fn a_take_home_could_not_hear_may_be_a_silence() {
		let essay = read(&body(&essay())).unwrap();
		let week = rust_answers(Some(&sent(&[("QM-311", "n1"), ("QM-312", "n2")])));
		let unheard = |takes: &[&'static str]| Final {
			unheard: takes.to_vec(),
			..rust_final(&week)
		};
		assert_eq!(
			against(&essay, &unheard(&["n2"])).unwrap(),
			[Played {
				ticket: String::from("QM-311"),
				take: String::from("n1"),
			}],
			"QM-312's garbled take, treated as unanswered"
		);
		assert!(against(&essay, &unheard(&["n1"])).is_err(), "QM-312 was heard: a silence for it is still refused");
		let mut both = essay;
		both.tickets[1].1 = Answered::Take;
		assert_eq!(against(&both, &unheard(&["n2"])).unwrap().len(), 2, "an unheard take may still be played");
	}

	fn interview_essay() -> serde_json::Value {
		json!({
			"schema": "some-ui.interview-essay/1",
			"week": { "id": "2026-w47" },
			"questions": [
				{ "id": "about-you", "answer": { "kind": "take" } },
				{ "id": "a-conflict", "answer": { "kind": "silence" } },
			],
			"reads": [{ "id": "intro", "answer": { "kind": "take" } }],
		})
	}

	fn interview_sent(answers: &[(&str, &str)]) -> crate::interview::Sent {
		crate::interview::Sent {
			week: String::from("2026-w47"),
			at: String::from("2026-11-22T20:00:00.000Z"),
			answers: answers
				.iter()
				.map(|(question, take)| crate::interview::Answer {
					question: (*question).to_owned(),
					take: (*take).to_owned(),
					recorded_at: String::from("2026-11-22T19:00:00.000Z"),
					rung: None,
				})
				.collect(),
			reads: vec![crate::interview::Read {
				card: String::from("intro"),
				take: String::from("note-read"),
				recorded_at: String::from("2026-11-22T19:00:00.000Z"),
			}],
			passed: Vec::new(),
			passed_reads: Vec::new(),
		}
	}

	#[test]
	fn an_interview_essay_is_named_and_read_for_its_week_and_each_questions_answer() {
		assert_eq!(parse_name_of(Kind::Interview, "interview-essay-2026-w47.json"), Some((String::from("2026-w47"), 1)));
		assert_eq!(parse_name_of(Kind::Interview, "interview-essay-2026-w47-r3.json"), Some((String::from("2026-w47"), 3)));
		for not in ["rust-essay-2026-w47.json", "interview-week-2026-w47.json", "interview-essay-2026-w47-r1.json"] {
			assert_eq!(parse_name_of(Kind::Interview, not), None, "{not}");
		}
		assert_eq!(parse_name_of(Kind::RustRounds, "interview-essay-2026-w47.json"), None, "each kind reads its own names");
		assert_eq!(
			read_of(Kind::Interview, &body(&interview_essay())).unwrap(),
			Essay {
				kind: Kind::Interview,
				week: String::from("2026-w47"),
				tickets: vec![(String::from("about-you"), Answered::Take), (String::from("a-conflict"), Answered::Silence)],
				points: Vec::new(),
				reads: vec![(String::from("intro"), Answered::Take)],
			}
		);
		assert!(read_of(Kind::RustRounds, &body(&interview_essay())).is_err(), "not a Rust Rounds essay");
		assert!(read_of(Kind::Interview, &body(&essay())).is_err(), "not an interview essay");
		let mut wrong = interview_essay();
		wrong["questions"][0]["id"] = json!("QM 311");
		wrong["questions"][1]["id"] = json!("QM 311");
		wrong["reads"][0]["answer"]["kind"] = json!("maybe");
		let fields: Vec<_> = read_of(Kind::Interview, &body(&wrong)).unwrap_err().into_iter().map(|(field, _)| field).collect();
		assert_eq!(fields, ["questions[0].id", "questions[1].id", "reads[0].answer.kind"]);
		let mut no_reads = interview_essay();
		no_reads.as_object_mut().unwrap().remove("reads");
		assert_eq!(read_of(Kind::Interview, &body(&no_reads)).unwrap().reads, [], "an essay of a week no card was read of");
		no_reads["reads"] = json!("intro");
		assert!(read_of(Kind::Interview, &body(&no_reads)).is_err());
		let mut with_reads = essay();
		with_reads["reads"] = json!("anything");
		assert!(read(&body(&with_reads)).is_ok(), "a Rust Rounds essay has no reads: the field is not read");
	}

	#[test]
	fn an_interview_essay_plays_exactly_the_answers_and_reads_the_weeks_final_names() {
		let essay = read_of(Kind::Interview, &body(&interview_essay())).unwrap();
		assert_eq!(
			against(&essay, &interview_final(Some(&interview_sent(&[("about-you", "note-1")])))).unwrap(),
			[
				Played {
					ticket: String::from("about-you"),
					take: String::from("note-1"),
				},
				Played {
					ticket: String::from("read.intro"),
					take: String::from("note-read"),
				}
			],
			"the question's take, then the card's read, named apart"
		);
		let fields = |sent: Option<&crate::interview::Sent>| -> Vec<String> { against(&essay, &interview_final(sent)).unwrap_err().into_iter().map(|(field, _)| field).collect() };
		assert_eq!(
			fields(None),
			["questions[0].answer.kind", "reads[0].answer.kind"],
			"a question nothing answered is a silence"
		);
		let mut unread = interview_sent(&[("about-you", "n1")]);
		unread.reads.clear();
		assert_eq!(fields(Some(&unread)), ["reads[0].answer.kind"], "a card the final never read is no take");
		let mut read_more = interview_sent(&[("about-you", "n1")]);
		read_more.reads.push(crate::interview::Read {
			card: String::from("outro"),
			take: String::from("n9"),
			recorded_at: String::from("2026-11-22T19:00:00.000Z"),
		});
		assert_eq!(fields(Some(&read_more)), ["reads[outro]"], "a card read and left out");
		assert_eq!(fields(Some(&interview_sent(&[("about-you", "n1"), ("a-conflict", "n2")]))), ["questions[1].answer.kind"]);
		assert_eq!(fields(Some(&interview_sent(&[("about-you", "n1"), ("why-us", "n3")]))), ["questions[why-us]"]);
	}

	/// HEARD1, for a week of mock interview: an unheard answer or read may be
	/// a silence.
	#[test]
	fn an_interview_answer_or_read_home_could_not_hear_may_be_a_silence() {
		let mut essay = read_of(Kind::Interview, &body(&interview_essay())).unwrap();
		essay.reads[0].1 = Answered::Silence;
		let week = interview_sent(&[("about-you", "n1"), ("a-conflict", "n2")]);
		let sent = Final {
			unheard: vec!["n2", "note-read"],
			..interview_final(Some(&week))
		};
		assert_eq!(
			against(&essay, &sent).unwrap(),
			[Played {
				ticket: String::from("about-you"),
				take: String::from("n1"),
			}]
		);
		let heard = interview_final(Some(&week));
		let fields: Vec<_> = against(&essay, &heard).unwrap_err().into_iter().map(|(field, _)| field).collect();
		assert_eq!(fields, ["questions[1].answer.kind", "reads[0].answer.kind"], "heard takes stay takes");
	}
}
