//! What home reads of a round: the least it needs to keep it and to record
//! its programs, and nothing more (paulgsc/some-ui#2234, RR3).
//!
//! The format is paulgsc/some-ui's `some-ui.rust-round/1`
//! (`packages/ui/rust-rounds/src/lib/formats`, RR1), and the phone's intake
//! holds a round to all of it before it is sent. Home checks only:
//!
//! - its size ([`ROUND_CEILING`]), its encoding and that it is one JSON object;
//! - its `schema` tag;
//! - its `id`, which names it for good;
//! - its cards' kinds and ids, and every program to run: a read card's
//!   `program`, and a variant card's, built as the format builds it
//!   ([`programs`]), so home records exactly the program the phone shows.
//!
//! Anything else in a round (a card's question, choices, guess, a refine
//! card's note and nudge) is kept as it came and never read here.

use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// The format's tag.
pub const ROUND_SCHEMA: &str = "some-ui.rust-round/1";

/// The most bytes a round may hold.
///
/// A round is a handful of cards, each program at most 25 lines (the
/// tutor's instructions): a few KiB. Every program a round holds is built
/// from the round's own text, so none is longer than this, and each fits the
/// runner's 64 KiB request with room to spare.
pub const ROUND_CEILING: usize = 32 * 1024;

/// The most characters a round's or a card's id may hold.
const MAX_ID_CHARS: usize = 64;

/// A field and why it was refused. The field is a path into the round
/// (`cards[2].hunk.line`), the reason fixed words.
pub type Problem = (String, &'static str);

/// One program to run: a read card's, or a variant card's as built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
	/// The card's id.
	pub card: String,
	/// `main.rs`, exactly.
	pub source: String,
}

/// A round home can keep: its id, its text, and its programs in card order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Round {
	pub id: String,
	/// The round's JSON as it came, JSON whitespace around it trimmed.
	pub text: String,
	pub programs: Vec<Program>,
}

/// Read `body` as a round, or say every problem found.
///
/// # Errors
/// Each [`Problem`]: a body past [`ROUND_CEILING`], not UTF-8 or not one JSON
/// object; a missing or other `schema`; an `id` that is not lower-case words
/// joined by `-`; no `cards`; a card whose kind is not `read`, `variant` or
/// `refine`, whose id is not a slug or is used twice, a read card with no
/// `program`, and a variant card whose program cannot be built (its `of`
/// names no earlier read or variant card, or its `hunk.line` is past the end
/// of that card's program, or `hunk.replace` is not one line).
pub fn read(body: &[u8]) -> Result<Round, Vec<Problem>> {
	let problem = |field: &str, why| vec![(field.to_owned(), why)];
	if body.len() > ROUND_CEILING {
		return Err(problem("body", "is larger than 32 KiB"));
	}
	let Ok(text) = std::str::from_utf8(body) else {
		return Err(problem("body", "is not UTF-8"));
	};
	let text = text.trim_matches([' ', '\t', '\n', '\r']);
	let Ok(Value::Object(round)) = serde_json::from_str::<Value>(text) else {
		return Err(problem("body", "is not one JSON object"));
	};
	let mut problems = Vec::new();
	if round.get("schema").and_then(Value::as_str) != Some(ROUND_SCHEMA) {
		problems.push((String::from("schema"), "must be \"some-ui.rust-round/1\""));
	}
	let id = round.get("id").and_then(Value::as_str).filter(|id| is_slug(id));
	if id.is_none() {
		problems.push((String::from("id"), "must be lower-case letters, digits and single hyphens"));
	}
	let programs = match round.get("cards").and_then(Value::as_array) {
		Some(cards) if !cards.is_empty() => programs(cards, &mut problems),
		_ => {
			problems.push((String::from("cards"), "must hold at least one card"));
			Vec::new()
		}
	};
	match id {
		Some(id) if problems.is_empty() => Ok(Round {
			id: id.to_owned(),
			text: text.to_owned(),
			programs,
		}),
		_ => Err(problems),
	}
}

/// Every read and variant card's program, in card order, as the format
/// builds them (`resolvePrograms`): a read card's own, and a variant's the
/// program of `of` with line `hunk.line` (1-based) replaced by
/// `hunk.replace`, keeping that program's final newline, if any.
fn programs(cards: &[Value], problems: &mut Vec<Problem>) -> Vec<Program> {
	let mut built: BTreeMap<&str, String> = BTreeMap::new();
	let mut seen = std::collections::BTreeSet::new();
	let mut programs = Vec::new();
	for (index, card) in cards.iter().enumerate() {
		let at = |field: &str| String::from("cards[") + &index.to_string() + "]" + field;
		let Some(card) = card.as_object() else {
			problems.push((at(""), "must be an object"));
			continue;
		};
		let Some(id) = card.get("id").and_then(Value::as_str).filter(|id| is_slug(id)) else {
			problems.push((at(".id"), "must be lower-case letters, digits and single hyphens"));
			continue;
		};
		if !seen.insert(id) {
			problems.push((at(".id"), "is used by an earlier card"));
			continue;
		}
		let source = match card.get("kind").and_then(Value::as_str) {
			Some("read") => {
				let Some(program) = card.get("program").and_then(Value::as_str) else {
					problems.push((at(".program"), "must be the program, as text"));
					continue;
				};
				program.to_owned()
			}
			Some("variant") => match variant(card, &built) {
				Ok(source) => source,
				Err((field, why)) => {
					problems.push((at(field), why));
					continue;
				}
			},
			Some("refine") => continue,
			_ => {
				problems.push((at(".kind"), "must be read, variant or refine"));
				continue;
			}
		};
		built.insert(id, source.clone());
		programs.push(Program { card: id.to_owned(), source });
	}
	programs
}

/// A variant card's program, or the field that keeps it from being built.
fn variant(card: &Map<String, Value>, built: &BTreeMap<&str, String>) -> Result<String, (&'static str, &'static str)> {
	let base = card
		.get("of")
		.and_then(Value::as_str)
		.and_then(|of| built.get(of))
		.ok_or((".of", "must name an earlier read or variant card in this round"))?;
	let hunk = card.get("hunk").and_then(Value::as_object).ok_or((".hunk", "must be an object"))?;
	let line = hunk
		.get("line")
		.and_then(Value::as_u64)
		.filter(|line| *line >= 1)
		.ok_or((".hunk.line", "must be a whole number from 1"))?;
	let replace = hunk
		.get("replace")
		.and_then(Value::as_str)
		.filter(|replace| !replace.contains(['\r', '\n']))
		.ok_or((".hunk.replace", "must be one whole line"))?;
	let mut lines = lines(base);
	let slot = usize::try_from(line - 1)
		.ok()
		.and_then(|at| lines.get_mut(at))
		.ok_or((".hunk.line", "is past the end of the program of `of`"))?;
	*slot = replace;
	let mut source = lines.join("\n");
	if base.ends_with('\n') {
		source.push('\n');
	}
	Ok(source)
}

/// A program's lines, as the format splits them (`programLines`): on `\n`,
/// a final newline ending the last line rather than starting another.
fn lines(program: &str) -> Vec<&str> {
	let mut lines: Vec<&str> = program.split('\n').collect();
	if lines.len() > 1 && lines.last() == Some(&"") {
		lines.pop();
	}
	lines
}

/// Lower-case letters and digits in words joined by single hyphens, as the
/// format's ids are.
fn is_slug(id: &str) -> bool {
	!id.is_empty()
		&& id.chars().count() <= MAX_ID_CHARS
		&& id
			.split('-')
			.all(|word| !word.is_empty() && word.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
	use super::{read, Program, ROUND_CEILING};
	use serde_json::json;

	fn body(cards: &serde_json::Value) -> Vec<u8> {
		json!({
			"schema": "some-ui.rust-round/1",
			"id": "indexing-01",
			"by": "claude-routine",
			"concept": "indexing",
			"edges": [],
			"cards": cards,
		})
		.to_string()
		.into_bytes()
	}

	const PROGRAM: &str = "fn main() {\n    let s = String::from(\"aéz\");\n    let r = 1;\n    let c = s[r];\n}\n";

	/// The worked example's shape: a read card, a variant of it, a variant
	/// of that, and a refine card, which holds no program.
	#[test]
	fn a_rounds_programs_are_built_as_the_format_builds_them() {
		let cards = json!([
			{ "kind": "read", "id": "a", "program": PROGRAM, "ask": "?", "choices": [], "expect": { "outcome": "panics" } },
			{ "kind": "variant", "id": "b", "of": "a", "hunk": { "line": 4, "replace": "    let c = s.as_bytes()[r];" } },
			{ "kind": "variant", "id": "c", "of": "b", "hunk": { "line": 1, "replace": "fn main() { // again" } },
			{ "kind": "refine", "id": "d", "note": "n1", "nudge": "x", "aim": "apply", "say": "y" },
		]);
		let round = read(&body(&cards)).unwrap();
		assert_eq!(round.id, "indexing-01");
		let b = "fn main() {\n    let s = String::from(\"aéz\");\n    let r = 1;\n    let c = s.as_bytes()[r];\n}\n";
		let c = "fn main() { // again\n    let s = String::from(\"aéz\");\n    let r = 1;\n    let c = s.as_bytes()[r];\n}\n";
		let expected = [("a", PROGRAM), ("b", b), ("c", c)].map(|(card, source)| Program {
			card: card.to_owned(),
			source: source.to_owned(),
		});
		assert_eq!(round.programs, expected);

		let unterminated = json!([
			{ "kind": "read", "id": "a", "program": "x\ny" },
			{ "kind": "variant", "id": "b", "of": "a", "hunk": { "line": 2, "replace": "z" } },
		]);
		assert_eq!(read(&body(&unterminated)).unwrap().programs[1].source, "x\nz", "no newline is added");
	}

	#[test]
	fn whitespace_around_the_json_is_no_part_of_it() {
		let mut padded = b"\n  ".to_vec();
		padded.extend(body(&json!([{ "kind": "read", "id": "a", "program": "fn main() {}" }])));
		padded.extend(b"\n");
		let round = read(&padded).unwrap();
		assert!(round.text.starts_with('{') && round.text.ends_with('}'));
	}

	fn refused(body: &[u8]) -> Vec<String> {
		read(body).unwrap_err().into_iter().map(|(field, _)| field).collect()
	}

	#[test]
	fn what_home_cannot_keep_or_run_is_refused_by_field() {
		assert_eq!(refused(&vec![b' '; ROUND_CEILING + 1]), ["body"]);
		assert_eq!(refused(&[0xff, 0xfe]), ["body"]);
		assert_eq!(refused(b"[1, 2]"), ["body"]);
		assert_eq!(refused(br#"{"schema":"some-ui.rust-round/2","id":"Bad Id","cards":[]}"#), ["schema", "id", "cards"]);
		let cards = json!([
			{ "kind": "read", "id": "a", "program": "fn main() {}\n" },
			{ "kind": "read", "id": "a", "program": "fn main() {}\n" },
			{ "kind": "read", "id": "b" },
			{ "kind": "variant", "id": "c", "of": "later", "hunk": { "line": 1, "replace": "x" } },
			{ "kind": "variant", "id": "d", "of": "a", "hunk": { "line": 2, "replace": "x" } },
			{ "kind": "variant", "id": "e", "of": "a", "hunk": { "line": 0, "replace": "x" } },
			{ "kind": "variant", "id": "f", "of": "a", "hunk": { "line": 1, "replace": "x\ny" } },
			{ "kind": "variant", "id": "g", "of": "d", "hunk": { "line": 1, "replace": "x" } },
			{ "kind": "grade", "id": "h" },
			{ "kind": "read", "id": "-i", "program": "" },
			"not a card",
			{ "kind": "variant", "id": "later", "of": "a", "hunk": { "line": 1, "replace": "x" } },
		]);
		assert_eq!(
			refused(&body(&cards)),
			[
				"cards[1].id",
				"cards[2].program",
				"cards[3].of",
				"cards[4].hunk.line",
				"cards[5].hunk.line",
				"cards[6].hunk.replace",
				"cards[7].of",
				"cards[8].kind",
				"cards[9].id",
				"cards[10]",
			]
		);
	}
}
