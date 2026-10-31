//! What a session-log event is in its home form, and the checks a write
//! passes before it is stored. Pure: no database.
//!
//! The shape is the phone's `SessionEventSchema`
//! (`packages/ui/leetype/src/lib/leetype/session-log/model.ts` in
//! paulgsc/some-ui), field for field and in its camelCase, with one
//! difference: the home form. Soundbites never go home, so a spoken answer
//! comes without its `ticks` (they go to the owner's Drive in the take note),
//! and no event may carry a [`SOUNDBITES`] field. What comes home of a spoken
//! answer is `as`, `seconds` and `strong`, which is what the ledger reads.
//!
//! Numbers are kept as the phone wrote them ([`Number`], parsed with
//! `float_roundtrip`), so an event reads back as the same JSON value it was
//! sent as: `23` stays `23`, never `23.0`, and a float keeps every digit. Key
//! order is not kept.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

/// `(field, why)`: one reason a write was refused.
pub type Problem = (&'static str, &'static str);

/// The most bytes one event's body may hold.
///
/// An event is a few hundred bytes: ids, a file and screen name, an option's
/// label. The session file the names come from has no length limits of its
/// own, so the ceiling is the Jobs record's, far above anything a session
/// file's author writes, and bounds a body all the same.
pub const EVENT_BODY_CEILING: usize = 256 * 1024;

/// The fields that hold a soundbite: what was said or typed, and what it was
/// marked against. They go to the owner's Drive, never home, so an event
/// carrying any of them is refused, by name.
pub const SOUNDBITES: [&str; 4] = ["ticks", "prompt", "rubric", "typed"];

const MAX_ID_CHARS: usize = 64;

/// How much of its session file a session plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Form {
	Full,
	Short,
}

/// Prop. 9.1's three responses to a round, kept as three. Slowness annotates
/// a correct or an incorrect pick, never the abstention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "response", rename_all = "lowercase", deny_unknown_fields)]
pub enum Answer {
	Abstained {},
	Correct { slow: bool },
	Incorrect { slow: bool },
}

/// Why a reading was offered: placed after a struggle, or a pointer after a
/// solved round.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Offered {
	Placement,
	Pointer,
}

/// What was done with a reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReadingAction {
	Opened,
	Saved,
	Read,
	Skipped,
}

/// A spoken answer, in its home form: passed, or answered aloud or typed and
/// then ticked against the rubric, `strong` when the ticks reached what the
/// screen asks. The ticks themselves stay off home.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "as", rename_all = "lowercase", deny_unknown_fields)]
pub enum SpokenAnswer {
	Passed {},
	Recorded { seconds: Number, strong: bool },
	Typed { strong: bool },
}

/// How a session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum How {
	Declared,
	Left,
}

/// One line of the phone's session log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Event {
	#[serde(rename_all = "camelCase")]
	Started {
		id: String,
		at: String,
		session: String,
		/// The session file the session plays.
		file: String,
		form: Form,
		declared_minutes: Number,
	},
	Picked {
		id: String,
		at: String,
		session: String,
		file: String,
		screen: String,
		/// The option's label.
		option: String,
		ms: Number,
		answer: Answer,
	},
	/// A reading offered for a round. `reading` is in `file` too.
	Placed {
		id: String,
		at: String,
		session: String,
		file: String,
		screen: String,
		reading: String,
		#[serde(rename = "as")]
		offered: Offered,
	},
	Reading {
		id: String,
		at: String,
		session: String,
		file: String,
		reading: String,
		action: ReadingAction,
	},
	Parked {
		id: String,
		at: String,
		session: String,
		file: String,
		screen: String,
	},
	/// The worked confirmation of a parked round, taken.
	ConfirmationTaken {
		id: String,
		at: String,
		session: String,
		file: String,
		screen: String,
	},
	/// A take's rubric shown that states the answer of `screen`, a round: a
	/// return of that round after it, until the round parks again, is
	/// assisted. `take` is the record screen whose rubric it was. Ids only:
	/// the rubric itself is a soundbite, and never comes home.
	RubricShown {
		id: String,
		at: String,
		session: String,
		file: String,
		screen: String,
		take: String,
	},
	/// A return played: `screen` is the parked round, `variant` the return,
	/// `asked` when the learner requested it.
	Returned {
		id: String,
		at: String,
		session: String,
		file: String,
		screen: String,
		option: String,
		ms: Number,
		answer: Answer,
		variant: String,
		asked: bool,
	},
	Spoken {
		id: String,
		at: String,
		session: String,
		file: String,
		screen: String,
		answer: SpokenAnswer,
	},
	Ended {
		id: String,
		at: String,
		session: String,
		file: String,
		screen: String,
		how: How,
	},
}

impl Event {
	/// The phone's id for it.
	#[must_use]
	pub fn id(&self) -> &str {
		match self {
			Self::Started { id, .. }
			| Self::Picked { id, .. }
			| Self::Placed { id, .. }
			| Self::Reading { id, .. }
			| Self::Parked { id, .. }
			| Self::ConfirmationTaken { id, .. }
			| Self::RubricShown { id, .. }
			| Self::Returned { id, .. }
			| Self::Spoken { id, .. }
			| Self::Ended { id, .. } => id,
		}
	}

	/// When the phone recorded it, as written.
	#[must_use]
	pub fn at(&self) -> &str {
		match self {
			Self::Started { at, .. }
			| Self::Picked { at, .. }
			| Self::Placed { at, .. }
			| Self::Reading { at, .. }
			| Self::Parked { at, .. }
			| Self::ConfirmationTaken { at, .. }
			| Self::RubricShown { at, .. }
			| Self::Returned { at, .. }
			| Self::Spoken { at, .. }
			| Self::Ended { at, .. } => at,
		}
	}

	/// The phone's id for the session it belongs to.
	#[must_use]
	pub fn session(&self) -> &str {
		match self {
			Self::Started { session, .. }
			| Self::Picked { session, .. }
			| Self::Placed { session, .. }
			| Self::Reading { session, .. }
			| Self::Parked { session, .. }
			| Self::ConfirmationTaken { session, .. }
			| Self::RubricShown { session, .. }
			| Self::Returned { session, .. }
			| Self::Spoken { session, .. }
			| Self::Ended { session, .. } => session,
		}
	}

	/// Its `type`, as the wire names it.
	#[must_use]
	pub const fn kind(&self) -> &'static str {
		match self {
			Self::Started { .. } => "started",
			Self::Picked { .. } => "picked",
			Self::Placed { .. } => "placed",
			Self::Reading { .. } => "reading",
			Self::Parked { .. } => "parked",
			Self::ConfirmationTaken { .. } => "confirmation-taken",
			Self::RubricShown { .. } => "rubric-shown",
			Self::Returned { .. } => "returned",
			Self::Spoken { .. } => "spoken",
			Self::Ended { .. } => "ended",
		}
	}
}

/// The phone's own ids (`crypto.randomUUID()` today): URL-safe, short.
fn is_id(text: &str) -> bool {
	!text.is_empty() && text.len() <= MAX_ID_CHARS && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// `at` as ISO-8601 UTC to the millisecond: the column a read orders by.
///
/// `None` when it is not an RFC 3339 instant in UTC (`Z`, as the phone's
/// `toISOString()` writes and its schema's `z.iso.datetime()` requires).
#[must_use]
pub fn instant(at: &str) -> Option<String> {
	if !at.ends_with('Z') {
		return None;
	}
	DateTime::parse_from_rfc3339(at)
		.ok()
		.map(|at| at.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn named(problems: &mut Vec<Problem>, field: &'static str, text: &str) {
	if text.is_empty() {
		problems.push((field, "must not be empty"));
	}
}

fn positive(problems: &mut Vec<Problem>, field: &'static str, value: &Number, why: &'static str) {
	if !value.as_f64().is_some_and(|value| value > 0.0) {
		problems.push((field, why));
	}
}

fn not_negative(problems: &mut Vec<Problem>, field: &'static str, value: &Number, why: &'static str) {
	if !value.as_f64().is_some_and(|value| value >= 0.0) {
		problems.push((field, why));
	}
}

/// The problems with one event's payload, by its type.
fn payload(event: &Event, problems: &mut Vec<Problem>) {
	match event {
		Event::Started { file, declared_minutes, .. } => {
			named(problems, "file", file);
			positive(problems, "declaredMinutes", declared_minutes, "is a number of minutes, more than 0");
		}
		Event::Picked { file, screen, ms, .. } => {
			named(problems, "file", file);
			named(problems, "screen", screen);
			not_negative(problems, "ms", ms, "is a number of milliseconds, 0 or more");
		}
		Event::Placed { file, screen, reading, .. } => {
			named(problems, "file", file);
			named(problems, "screen", screen);
			named(problems, "reading", reading);
		}
		Event::Reading { file, reading, .. } => {
			named(problems, "file", file);
			named(problems, "reading", reading);
		}
		Event::Parked { file, screen, .. } | Event::ConfirmationTaken { file, screen, .. } | Event::Ended { file, screen, .. } => {
			named(problems, "file", file);
			named(problems, "screen", screen);
		}
		Event::RubricShown { file, screen, take, .. } => {
			named(problems, "file", file);
			named(problems, "screen", screen);
			named(problems, "take", take);
		}
		Event::Returned { file, screen, ms, variant, .. } => {
			named(problems, "file", file);
			named(problems, "screen", screen);
			not_negative(problems, "ms", ms, "is a number of milliseconds, 0 or more");
			named(problems, "variant", variant);
		}
		Event::Spoken { file, screen, answer, .. } => {
			named(problems, "file", file);
			named(problems, "screen", screen);
			if let SpokenAnswer::Recorded { seconds, .. } = answer {
				positive(problems, "answer.seconds", seconds, "is a number of seconds, more than 0");
			}
		}
	}
}

/// Takes every soundbite field out of `event`, an event's JSON object, and
/// names each one taken: at the top level, and in a spoken answer.
fn take_soundbites(event: &mut Map<String, Value>, problems: &mut Vec<Problem>) {
	const WHY: &str = "is a soundbite: it goes to the owner's Drive, never home";
	const IN_ANSWER: [&str; 4] = ["answer.ticks", "answer.prompt", "answer.rubric", "answer.typed"];
	for field in SOUNDBITES {
		if event.remove(field).is_some() {
			problems.push((field, WHY));
		}
	}
	if event.get("type").and_then(Value::as_str) != Some("spoken") {
		return;
	}
	if let Some(Value::Object(answer)) = event.get_mut("answer") {
		for (field, name) in SOUNDBITES.into_iter().zip(IN_ANSWER) {
			if answer.remove(field).is_some() {
				problems.push((name, WHY));
			}
		}
	}
}

/// The event in `body`, checked.
///
/// It must be the one `session` and `id` name, carry no soundbite, and hold
/// every rule its type holds.
///
/// # Errors
/// Every problem found, as `(field, why)`.
pub fn parse_event(session: &str, id: &str, body: &[u8]) -> Result<Event, Vec<Problem>> {
	const OFF_SHAPE: Problem = ("body", "is not the shape this record holds");
	if body.len() > EVENT_BODY_CEILING {
		return Err(vec![("body", "is over the size ceiling")]);
	}
	let Ok(Value::Object(mut object)) = serde_json::from_slice::<Value>(body) else {
		return Err(vec![OFF_SHAPE]);
	};
	let mut problems = Vec::new();
	take_soundbites(&mut object, &mut problems);
	// Read from the bytes when nothing was taken out, so a key written twice
	// is refused rather than read as its last copy.
	let parsed = if problems.is_empty() {
		serde_json::from_slice::<Event>(body)
	} else {
		serde_json::from_value::<Event>(Value::Object(object))
	};
	let Ok(event) = parsed else {
		problems.push(OFF_SHAPE);
		return Err(problems);
	};
	if !is_id(id) {
		problems.push(("id", "must be the phone's id for the event: 1 to 64 letters, digits, `-` or `_`"));
	} else if event.id() != id {
		problems.push(("id", "must be the id the path names"));
	}
	if !is_id(session) {
		problems.push(("session", "must be the phone's id for the session: 1 to 64 letters, digits, `-` or `_`"));
	} else if event.session() != session {
		problems.push(("session", "must be the session the path names"));
	}
	if instant(event.at()).is_none() {
		problems.push(("at", "must be an RFC 3339 instant in UTC, ending `Z`"));
	}
	payload(&event, &mut problems);
	if problems.is_empty() {
		Ok(event)
	} else {
		Err(problems)
	}
}

#[cfg(test)]
mod tests {
	use super::{instant, parse_event, Event};
	use serde_json::{json, Value};

	const S: &str = "4f0c2a8e-7b1d-4c3e-9a55-0d1e2f3a4b5c";

	fn fields(session: &str, id: &str, value: &Value) -> Vec<&'static str> {
		let mut fields: Vec<&str> = parse_event(session, id, value.to_string().as_bytes())
			.unwrap_err()
			.iter()
			.map(|(field, _)| *field)
			.collect();
		fields.sort_unstable();
		fields
	}

	fn event(kind: &str, extra: &Value) -> Value {
		let mut event = json!({ "id": "e1", "at": "2026-10-20T09:15:00.000Z", "session": S, "type": kind });
		for (key, value) in extra.as_object().unwrap() {
			event[key] = value.clone();
		}
		event
	}

	fn spoken(answer: &Value) -> Value {
		event("spoken", &json!({ "file": "raft-1.json", "screen": "say-why", "answer": answer }))
	}

	#[test]
	fn every_phone_event_type_reads_and_writes_back_as_it_came() {
		let on = json!({ "file": "raft-1.json", "screen": "r1" });
		let with = |kind: &str, extra: Value| {
			let mut all = extra;
			for (key, value) in on.as_object().unwrap() {
				all[key] = value.clone();
			}
			event(kind, &all)
		};
		let events = [
			event("started", &json!({ "file": "raft-1.json", "form": "full", "declaredMinutes": 25 })),
			event("started", &json!({ "file": "raft-1.json", "form": "short", "declaredMinutes": 7.5 })),
			with("picked", json!({ "option": "Step down", "ms": 4210, "answer": { "response": "correct", "slow": false } })),
			with("picked", json!({ "option": "", "ms": 0, "answer": { "response": "abstained" } })),
			with("picked", json!({ "option": "Retry", "ms": 12_000.5, "answer": { "response": "incorrect", "slow": true } })),
			with("placed", json!({ "reading": "ongaro-5.2", "as": "placement" })),
			with("placed", json!({ "reading": "ongaro-5.2", "as": "pointer" })),
			event("reading", &json!({ "file": "raft-1.json", "reading": "ongaro-5.2", "action": "saved" })),
			with("parked", json!({})),
			with("confirmation-taken", json!({})),
			with("rubric-shown", json!({ "take": "say-why" })),
			with(
				"returned",
				json!({ "option": "Step down", "ms": 3000, "answer": { "response": "correct", "slow": false }, "variant": "r1-b", "asked": true }),
			),
			spoken(&json!({ "as": "passed" })),
			spoken(&json!({ "as": "recorded", "seconds": 41.2, "strong": true })),
			// `performance.now() / 1000`: 17 significant digits, the last one kept.
			spoken(&json!({ "as": "recorded", "seconds": 31.353_200_000_002_982, "strong": false })),
			spoken(&json!({ "as": "typed", "strong": false })),
			with("ended", json!({ "how": "declared" })),
			with("ended", json!({ "how": "left" })),
		];
		for event in events {
			let parsed = parse_event(S, "e1", event.to_string().as_bytes()).unwrap_or_else(|problems| panic!("{event}: {problems:?}"));
			// `Value`'s numbers compare by kind too: `25` is not `25.0`.
			assert_eq!(serde_json::to_value(&parsed).unwrap(), event, "{event}");
		}
		let whole = event("started", &json!({ "file": "f", "form": "full", "declaredMinutes": 25 }));
		let text = serde_json::to_value(parse_event(S, "e1", whole.to_string().as_bytes()).unwrap()).unwrap().to_string();
		assert!(text.contains(r#""declaredMinutes":25,"#), "a whole number stays whole: {text}");
	}

	/// The home form: what was said, typed or marked against never comes
	/// home, and each field that would carry it is named.
	#[test]
	fn a_soundbite_is_refused_by_name() {
		let recorded = json!({ "as": "recorded", "seconds": 41.2, "ticks": ["names the term"], "strong": true });
		assert_eq!(fields(S, "e1", &spoken(&recorded)), ["answer.ticks"], "the phone's own spoken answer, sent whole");
		let typed = json!({ "as": "typed", "ticks": [], "typed": "Because the leader…", "strong": false });
		assert_eq!(fields(S, "e1", &spoken(&typed)), ["answer.ticks", "answer.typed"]);
		let marked = json!({ "as": "recorded", "seconds": 3, "strong": true, "prompt": "Why?", "rubric": ["a"] });
		assert_eq!(fields(S, "e1", &spoken(&marked)), ["answer.prompt", "answer.rubric"]);
		let mut beside = spoken(&json!({ "as": "passed" }));
		beside["prompt"] = json!("Why?");
		beside["ticks"] = json!([]);
		assert_eq!(fields(S, "e1", &beside), ["prompt", "ticks"], "nor beside the answer");
		let mut both = spoken(&json!({ "as": "recorded", "seconds": 0, "strong": true, "ticks": [] }));
		both["screen"] = json!("");
		assert_eq!(fields(S, "e1", &both), ["answer.seconds", "answer.ticks", "screen"], "and the other problems still named");
		let mut off = spoken(&json!({ "as": "recorded", "ticks": [] }));
		off["mood"] = json!("tired");
		assert_eq!(fields(S, "e1", &off), ["answer.ticks", "body"]);
	}

	/// A take's rubric shown: the file, the round and the take, by name, and
	/// nothing else. What the rubric says stays off home like any soundbite.
	#[test]
	fn a_shown_rubric_comes_home_as_ids_alone() {
		let sent = r#"{"type":"rubric-shown","id":"e1","at":"2026-10-20T09:14:00.000Z","session":"4f0c2a8e-7b1d-4c3e-9a55-0d1e2f3a4b5c","file":"raft-1.json","screen":"r2","take":"say-why"}"#;
		let parsed = parse_event(S, "e1", sent.as_bytes()).unwrap();
		assert_eq!(parsed.kind(), "rubric-shown");
		let mut written = Vec::new();
		serde_json::to_writer(&mut written, &parsed).unwrap();
		assert_eq!(String::from_utf8(written).unwrap(), sent, "written back byte for byte");

		let shown = || event("rubric-shown", &json!({ "file": "raft-1.json", "screen": "r2", "take": "say-why" }));
		let mut extra = shown();
		extra["covers"] = json!(["r2"]);
		assert_eq!(fields(S, "e1", &extra), ["body"], "no field the phone does not send");
		let mut missing = shown();
		missing.as_object_mut().unwrap().remove("take");
		assert_eq!(fields(S, "e1", &missing), ["body"], "the take is named");
		let mut said = shown();
		said["rubric"] = json!(["names the term"]);
		said["ticks"] = json!([]);
		assert_eq!(fields(S, "e1", &said), ["rubric", "ticks"], "what the rubric says is a soundbite");
	}

	#[test]
	fn a_body_off_the_shape_is_refused_whole() {
		let picked = || {
			event(
				"picked",
				&json!({ "file": "f", "screen": "r1", "option": "A", "ms": 10, "answer": { "response": "correct", "slow": false } }),
			)
		};
		let mut extra = picked();
		extra["mood"] = json!("tired");
		assert_eq!(fields(S, "e1", &extra), ["body"], "no field the phone does not send");
		let mut unknown = picked();
		unknown["type"] = json!("deleted");
		assert_eq!(fields(S, "e1", &unknown), ["body"], "no type the phone does not record");
		let mut abstained = picked();
		abstained["answer"] = json!({ "response": "abstained", "slow": true });
		assert_eq!(fields(S, "e1", &abstained), ["body"], "an abstention is never slow");
		let mut missing = picked();
		missing.as_object_mut().unwrap().remove("ms");
		assert_eq!(fields(S, "e1", &missing), ["body"]);
		let mut form = event("started", &json!({ "file": "f", "form": "long", "declaredMinutes": 5 }));
		assert_eq!(fields(S, "e1", &form), ["body"], "forms come from the phone's list");
		form["form"] = json!("full");
		form["declaredMinutes"] = json!("5");
		assert_eq!(fields(S, "e1", &form), ["body"], "minutes are a number");
		let twice = picked().to_string().replacen("\"id\":\"e1\"", "\"id\":\"e1\",\"id\":\"e1\"", 1);
		assert!(parse_event(S, "e1", twice.as_bytes()).is_err(), "a key written twice");
		let big = json!({ "id": "e1", "padding": "x".repeat(super::EVENT_BODY_CEILING) });
		assert_eq!(fields(S, "e1", &big), ["body"]);
		assert_eq!(fields(S, "e1", &json!(["event"])), ["body"]);
	}

	#[test]
	fn each_field_is_held_to_the_phones_rules() {
		let wrong = event(
			"returned",
			&json!({ "at": "2026-10-20T02:15:00-07:00", "file": "", "screen": "", "option": "", "ms": -1, "answer": { "response": "correct", "slow": false }, "variant": "", "asked": false }),
		);
		assert_eq!(fields(S, "e1", &wrong), ["at", "file", "ms", "screen", "variant"]);
		let started = event("started", &json!({ "at": "yesterday", "file": "", "form": "full", "declaredMinutes": 0 }));
		assert_eq!(fields(S, "e1", &started), ["at", "declaredMinutes", "file"]);
		let placed = event("placed", &json!({ "file": "f", "screen": "r1", "reading": "", "as": "pointer" }));
		assert_eq!(fields(S, "e1", &placed), ["reading"]);
		let reading = event("reading", &json!({ "file": "", "reading": "", "action": "read" }));
		assert_eq!(fields(S, "e1", &reading), ["file", "reading"]);
		let shown = event("rubric-shown", &json!({ "file": "", "screen": "", "take": "" }));
		assert_eq!(fields(S, "e1", &shown), ["file", "screen", "take"]);

		let parked = event("parked", &json!({ "file": "f", "screen": "r1" }));
		assert_eq!(fields(S, "e2", &parked), ["id"], "the id the path names");
		assert_eq!(fields(S, "e/1", &parked), ["id"], "a plain id");
		assert_eq!(fields("another-session", "e1", &parked), ["session"], "the session the path names");
		assert_eq!(fields("a:b", "e1", &parked), ["session"], "a plain session id");
	}

	#[test]
	fn the_order_a_read_serves_is_utc_to_the_millisecond() {
		assert_eq!(instant("2026-10-20T09:15:00Z").as_deref(), Some("2026-10-20T09:15:00.000Z"));
		assert_eq!(instant("2026-10-20T09:15:00.5Z").as_deref(), Some("2026-10-20T09:15:00.500Z"));
		assert_eq!(instant("2026-10-20T02:15:00-07:00"), None, "the phone writes UTC");
		assert_eq!(instant("2026-10-20"), None);
		let event: Event = parse_event(S, "e1", event("parked", &json!({ "file": "f", "screen": "r1" })).to_string().as_bytes()).unwrap();
		assert_eq!((event.id(), event.at(), event.session(), event.kind()), ("e1", "2026-10-20T09:15:00.000Z", S, "parked"));
	}
}
