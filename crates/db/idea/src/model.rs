//! What an idea's send is in its home form, the lists' entries, and the
//! checks a write passes before it is stored. Pure: no database.
//!
//! The wire form is the one paulgsc/some-ui ST26 (#2453) sends: one JSON
//! object per send, its keys `camelCase`, each answer naming a voice note by
//! id or carrying the words typed when voice failed.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;

/// `(field, why)`: one reason a write was refused.
pub type Problem = (&'static str, &'static str);

/// The most bytes one send's body may hold.
///
/// A send is a title, a few ids and at most four typed answers of at most
/// [`MAX_TYPED_CHARS`] characters each: well under this. Every field is
/// bounded on its own, so this only stops a body from being read whole before
/// it is refused.
pub const BODY_CEILING: usize = 64 * 1024;

/// The most characters a title holds, after trimming (the owner, on
/// paulgsc/some-ui#2453: "we don't expect title to be some essay").
pub const MAX_TITLE_CHARS: usize = 80;

/// The most characters an answer typed when voice failed holds.
pub const MAX_TYPED_CHARS: usize = 1_000;

/// The most keywords one idea carries.
pub const MAX_KEYWORDS: usize = 20;

/// The most characters one entry of a list holds.
pub const MAX_ENTRY_CHARS: usize = 40;

/// The most entries one list holds.
pub const MAX_LIST_ENTRIES: usize = 100;

const MAX_ID_CHARS: usize = 64;

/// The question sets home knows, by version: each set's question ids in the
/// order the phone asks them, the first being the one every send answers.
///
/// The phone's own copy is `IDEA_QUESTIONS` (paulgsc/some-ui
/// `packages/ui/soundbites/src/lib/idea/model.ts`). A new version there is a
/// new entry here, and an idea answering an older one still reads.
pub const QUESTION_SETS: &[(i64, &[&str])] = &[(1, &["what", "takeaway", "spark", "picture"])];

/// The topics every subject's list starts with (paulgsc/server#645).
pub const SEED_TOPICS: &[&str] = &["Studio", "LeetType", "Korean", "Playoff Delta", "Rust Rounds"];

/// The keywords every subject's list starts with (paulgsc/server#645).
pub const SEED_KEYWORDS: &[&str] = &["home", "voice", "phone", "mistakes", "how it works", "behind the scenes"];

/// The format an idea is for.
pub const FORMATS: [&str; 2] = ["short", "video"];

/// One of the lists the owner edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum List {
	Topic,
	Keyword,
}

impl List {
	/// The name the `list` column holds.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Topic => "topic",
			Self::Keyword => "keyword",
		}
	}

	/// The entries the list starts with.
	#[must_use]
	pub const fn seeds(self) -> &'static [&'static str] {
		match self {
			Self::Topic => SEED_TOPICS,
			Self::Keyword => SEED_KEYWORDS,
		}
	}
}

/// How an answer was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Said {
	/// A `studio` voice note, by the phone's id.
	Note(String),
	/// The words typed when voice failed.
	Typed(String),
}

/// One answer of a send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
	pub question: String,
	pub said: Said,
}

/// What a send says of the idea: everything but when it was sent. Two sends
/// with equal content are the same bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Content {
	/// ISO-8601 UTC to the millisecond.
	pub created_at: String,
	/// Trimmed.
	pub title: String,
	pub topic: String,
	pub format: String,
	pub keywords: Vec<String>,
	pub questions: i64,
	/// In the question set's order.
	pub answers: Vec<Answer>,
}

/// One send, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
	pub id: String,
	/// ISO-8601 UTC to the millisecond.
	pub sent_at: String,
	pub content: Content,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct WireAnswer {
	question: String,
	#[serde(default)]
	note: Option<String>,
	#[serde(default)]
	typed: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Wire {
	id: String,
	created_at: String,
	sent_at: String,
	title: String,
	topic: String,
	format: String,
	#[serde(default)]
	keywords: Vec<String>,
	questions: i64,
	answers: Vec<WireAnswer>,
}

/// The phone's own ids (`crypto.randomUUID()` today): URL-safe, short.
#[must_use]
pub fn is_id(text: &str) -> bool {
	!text.is_empty() && text.len() <= MAX_ID_CHARS && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// An entry of a list, as the owner writes it: 1 to [`MAX_ENTRY_CHARS`]
/// characters, with no space at either end, no control character and no `/`.
#[must_use]
pub fn is_entry(text: &str) -> bool {
	!text.is_empty() && text.trim() == text && text.chars().count() <= MAX_ENTRY_CHARS && !text.chars().any(|c| c.is_control() || c == '/')
}

/// `at` as ISO-8601 UTC to the millisecond, the form home keeps.
///
/// `None` when it is not an RFC 3339 instant in UTC (`Z`, as the phone's
/// `toISOString()` writes).
#[must_use]
pub fn instant(at: &str) -> Option<String> {
	if !at.ends_with('Z') {
		return None;
	}
	DateTime::parse_from_rfc3339(at)
		.ok()
		.map(|at| at.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Millis, true))
}

/// The question set of `version`, if home knows it.
#[must_use]
pub fn question_set(version: i64) -> Option<&'static [&'static str]> {
	QUESTION_SETS.iter().find(|(known, _)| *known == version).map(|(_, set)| *set)
}

fn is_typed(text: &str) -> bool {
	!text.trim().is_empty() && text.chars().count() <= MAX_TYPED_CHARS && !text.chars().any(|c| c.is_control() && c != '\n')
}

fn title(problems: &mut Vec<Problem>, sent: &str) -> String {
	let trimmed = sent.trim();
	if trimmed.is_empty() || trimmed.chars().count() > MAX_TITLE_CHARS || trimmed.chars().any(char::is_control) {
		problems.push(("title", "must be 1 to 80 characters once trimmed, on one line"));
	}
	trimmed.to_owned()
}

fn keywords(problems: &mut Vec<Problem>, sent: &[String]) {
	if sent.len() > MAX_KEYWORDS {
		problems.push(("keywords", "holds at most 20 entries"));
	}
	if !sent.iter().all(|keyword| is_entry(keyword)) {
		problems.push(("keywords", "each must be an entry of the keywords list"));
	}
	if sent.iter().enumerate().any(|(n, keyword)| sent[..n].contains(keyword)) {
		problems.push(("keywords", "names each entry once"));
	}
}

fn answers(problems: &mut Vec<Problem>, set: Option<&[&str]>, given: Vec<WireAnswer>) -> Vec<Answer> {
	let mut kept: Vec<Answer> = Vec::with_capacity(given.len());
	for answer in given {
		let said = match (answer.note, answer.typed) {
			(Some(note), None) => {
				if !voice_repo::is_note_id(&note) {
					problems.push(("answers", "a note must be the phone's id for a voice note"));
				}
				Said::Note(note)
			}
			(None, Some(typed)) => {
				if !is_typed(&typed) {
					problems.push(("answers", "a typed answer is 1 to 1,000 characters, not blank"));
				}
				Said::Typed(typed)
			}
			_ => {
				problems.push(("answers", "each answer names a voice note or is typed, not both"));
				continue;
			}
		};
		if kept.iter().any(|held| held.question == answer.question) {
			problems.push(("answers", "answers each question once"));
		}
		if set.is_some_and(|set| !set.contains(&answer.question.as_str())) {
			problems.push(("answers", "each answers a question of the idea's question set"));
		}
		kept.push(Answer { question: answer.question, said });
	}
	if let Some(set) = set {
		if !kept.iter().any(|answer| answer.question == set[0]) {
			problems.push(("answers", "must answer the first question"));
		}
		kept.sort_by_key(|answer| set.iter().position(|question| *question == answer.question));
	}
	kept
}

/// The send in `body`, checked against every rule that needs no database.
///
/// It must be the idea `id` names. What the lists hold and which voice notes
/// home holds are the store's to check.
///
/// # Errors
/// Every problem found, as `(field, why)`, each field once per reason.
pub fn parse_bundle(id: &str, body: &[u8]) -> Result<Bundle, Vec<Problem>> {
	if body.len() > BODY_CEILING {
		return Err(vec![("body", "is over the size ceiling")]);
	}
	// Straight from the bytes, so a key written twice is refused rather than
	// read as its last copy.
	let wire: Wire = serde_json::from_slice(body).map_err(|_| vec![("body", "is not the shape an idea's send holds")])?;
	let mut problems = Vec::new();
	if !is_id(id) {
		problems.push(("id", "must be the phone's id: 1 to 64 letters, digits, `-` or `_`"));
	} else if wire.id != id {
		problems.push(("id", "must be the id the path names"));
	}
	let created_at = instant(&wire.created_at).unwrap_or_else(|| {
		problems.push(("createdAt", "must be an RFC 3339 instant in UTC, ending `Z`"));
		String::new()
	});
	let sent_at = instant(&wire.sent_at).unwrap_or_else(|| {
		problems.push(("sentAt", "must be an RFC 3339 instant in UTC, ending `Z`"));
		String::new()
	});
	let title = title(&mut problems, &wire.title);
	if !is_entry(&wire.topic) {
		problems.push(("topic", "must be an entry of the topics list"));
	}
	if !FORMATS.contains(&wire.format.as_str()) {
		problems.push(("format", "must be `short` or `video`"));
	}
	keywords(&mut problems, &wire.keywords);
	let set = question_set(wire.questions);
	if set.is_none() {
		problems.push(("questions", "must be a question set's version home knows"));
	}
	let answers = answers(&mut problems, set, wire.answers);
	problems.dedup();
	if !problems.is_empty() {
		return Err(problems);
	}
	Ok(Bundle {
		id: wire.id,
		sent_at,
		content: Content {
			created_at,
			title,
			topic: wire.topic,
			format: wire.format,
			keywords: wire.keywords,
			questions: wire.questions,
			answers,
		},
	})
}

#[cfg(test)]
mod tests {
	use super::{instant, is_entry, parse_bundle, Said, BODY_CEILING, MAX_TITLE_CHARS, MAX_TYPED_CHARS, SEED_KEYWORDS, SEED_TOPICS};
	use serde_json::{json, Value};

	fn send() -> Value {
		json!({
			"id": "i1",
			"createdAt": "2026-11-01T09:00:00.000Z",
			"sentAt": "2026-11-01T09:10:00.000Z",
			"title": "Why the mic clips",
			"topic": "Studio",
			"format": "short",
			"keywords": ["voice", "mistakes"],
			"questions": 1,
			"answers": [
				{ "question": "takeaway", "typed": "Keep the phone a hand away." },
				{ "question": "what", "note": "n1" }
			]
		})
	}

	fn fields(value: &Value) -> Vec<&'static str> {
		let mut fields: Vec<&'static str> = parse_bundle("i1", value.to_string().as_bytes()).unwrap_err().iter().map(|(field, _)| *field).collect();
		fields.sort_unstable();
		fields.dedup();
		fields
	}

	#[test]
	fn a_send_reads_with_its_answers_in_the_question_sets_order() {
		let bundle = parse_bundle("i1", send().to_string().as_bytes()).unwrap();
		assert_eq!(bundle.id, "i1");
		assert_eq!(bundle.sent_at, "2026-11-01T09:10:00.000Z");
		assert_eq!(bundle.content.created_at, "2026-11-01T09:00:00.000Z");
		assert_eq!(bundle.content.keywords, ["voice", "mistakes"], "in the order sent");
		let questions: Vec<&str> = bundle.content.answers.iter().map(|answer| answer.question.as_str()).collect();
		assert_eq!(questions, ["what", "takeaway"], "in the set's order, however sent");
		assert_eq!(bundle.content.answers[0].said, Said::Note("n1".into()));
		assert_eq!(bundle.content.answers[1].said, Said::Typed("Keep the phone a hand away.".into()));

		let mut bare = send();
		bare.as_object_mut().unwrap().remove("keywords");
		bare["answers"] = json!([{ "question": "what", "note": "n1", "typed": null }]);
		let bundle = parse_bundle("i1", bare.to_string().as_bytes()).unwrap();
		assert!(bundle.content.keywords.is_empty(), "keywords may be left out");
		assert_eq!(bundle.content.answers.len(), 1, "only the first question has to be answered");
	}

	#[test]
	fn the_title_is_one_to_eighty_characters_once_trimmed() {
		let mut padded = send();
		padded["title"] = json!("   A title   ");
		assert_eq!(parse_bundle("i1", padded.to_string().as_bytes()).unwrap().content.title, "A title", "kept trimmed");
		let mut long = send();
		long["title"] = json!(String::from(" ") + &"한".repeat(MAX_TITLE_CHARS) + " ");
		assert!(parse_bundle("i1", long.to_string().as_bytes()).is_ok(), "80 characters, not bytes, once trimmed");
		long["title"] = json!("x".repeat(MAX_TITLE_CHARS + 1));
		assert_eq!(fields(&long), ["title"]);
		long["title"] = json!("   ");
		assert_eq!(fields(&long), ["title"], "blank");
		long["title"] = json!("one\ntwo");
		assert_eq!(fields(&long), ["title"], "one line");
	}

	#[test]
	fn a_typed_answer_is_at_most_a_thousand_characters() {
		let mut typed = send();
		typed["answers"][0]["typed"] = json!("é".repeat(MAX_TYPED_CHARS));
		assert!(parse_bundle("i1", typed.to_string().as_bytes()).is_ok(), "1,000 characters, not bytes");
		typed["answers"][0]["typed"] = json!("lines\nare fine");
		assert!(parse_bundle("i1", typed.to_string().as_bytes()).is_ok());
		typed["answers"][0]["typed"] = json!("x".repeat(MAX_TYPED_CHARS + 1));
		assert_eq!(fields(&typed), ["answers"]);
		typed["answers"][0]["typed"] = json!(" \n ");
		assert_eq!(fields(&typed), ["answers"], "blank");
	}

	#[test]
	fn the_first_question_must_be_answered_and_each_once() {
		let mut first = send();
		first["answers"] = json!([{ "question": "takeaway", "typed": "later" }]);
		assert_eq!(fields(&first), ["answers"]);
		first["answers"] = json!([]);
		assert_eq!(fields(&first), ["answers"]);
		let mut twice = send();
		twice["answers"] = json!([{ "question": "what", "note": "n1" }, { "question": "what", "typed": "again" }]);
		assert_eq!(fields(&twice), ["answers"]);
		let mut unknown = send();
		unknown["answers"][0]["question"] = json!("ask-1");
		assert_eq!(fields(&unknown), ["answers"], "only the set's questions");
		let mut both = send();
		both["answers"][1]["typed"] = json!("and typed");
		assert_eq!(fields(&both), ["answers"], "a note or typed words, not both");
		both["answers"][1] = json!({ "question": "what" });
		assert_eq!(fields(&both), ["answers"], "nor neither");
		both["answers"][1] = json!({ "question": "what", "note": "n/1" });
		assert_eq!(fields(&both), ["answers"], "a note's id");
	}

	#[test]
	fn each_other_field_is_held_to_its_rule() {
		let mut bad = send();
		bad["createdAt"] = json!("2026-11-01T02:00:00-07:00");
		bad["sentAt"] = json!("soon");
		bad["topic"] = json!("");
		bad["format"] = json!("reel");
		bad["keywords"] = json!(["voice", "voice"]);
		bad["questions"] = json!(9);
		assert_eq!(fields(&bad), ["createdAt", "format", "keywords", "questions", "sentAt", "topic"]);
		let mut many = send();
		many["keywords"] = json!((0..21).map(|n| n.to_string()).collect::<Vec<_>>());
		assert_eq!(fields(&many), ["keywords"]);
		for format in ["short", "video"] {
			let mut format_sent = send();
			format_sent["format"] = json!(format);
			assert!(parse_bundle("i1", format_sent.to_string().as_bytes()).is_ok(), "{format}");
		}
		assert_eq!(parse_bundle("i2", send().to_string().as_bytes()).unwrap_err(), [("id", "must be the id the path names")]);
		assert_eq!(fields(&json!(["idea"])), ["body"]);
		let mut extra = send();
		extra["furnishing"] = json!(true);
		assert_eq!(fields(&extra), ["body"], "no field the send does not carry");
		let big = json!({ "id": "i1", "padding": "x".repeat(BODY_CEILING) });
		assert_eq!(fields(&big), ["body"]);
		let twice = send().to_string().replacen("\"id\":\"i1\"", "\"id\":\"i1\",\"id\":\"i1\"", 1);
		assert!(parse_bundle("i1", twice.as_bytes()).is_err(), "a key written twice");
	}

	#[test]
	fn entries_and_instants() {
		for entry in SEED_TOPICS.iter().chain(SEED_KEYWORDS) {
			assert!(is_entry(entry), "{entry}");
		}
		for entry in ["", " voice", "voice ", "a/b", "tab\there", &"x".repeat(41)] {
			assert!(!is_entry(entry), "{entry:?}");
		}
		assert_eq!(instant("2026-11-01T09:15:00Z").as_deref(), Some("2026-11-01T09:15:00.000Z"));
		assert_eq!(instant("2026-11-01T02:15:00-07:00"), None);
	}
}
