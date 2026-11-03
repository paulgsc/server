//! What an app event is in its home form, and the checks a write passes
//! before it is stored. Pure: no database.
//!
//! The wire form is the one paulgsc/some-ui CK2 (#2381) sends: a plain JSON
//! object whose `family` says which shape the rest has. Every text field is
//! an id from the app's own code (a process family, a screen, an ending), so
//! nothing typed or said fits in one (the owner's C3, paulgsc/some-ui#2369).
//! An optional field the phone leaves out reads back as `null`, as one it
//! sends as `null` does, so a resend either way compares equal.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

/// `(field, why)`: one reason a write was refused.
pub type Problem = (&'static str, &'static str);

/// The most bytes one event's body may hold.
///
/// An outcome is about 250 bytes. Every field is bounded on its own, so this
/// only stops a body from being read whole before it is refused.
pub const BODY_CEILING: usize = 4 * 1024;

const MAX_ID_CHARS: usize = 64;

/// The longest time an outcome may say a step took: a day. A take is capped
/// at minutes; anything longer is a clock that jumped, not a take.
pub const MAX_MS: u32 = 86_400_000;

/// How an ending reads, as the phone's own table for its process says: it
/// worked, it failed, or the person stopped it (a take too short to keep, a
/// take let go).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Went {
	Worked,
	Failed,
	Stopped,
}

impl Went {
	/// The wire name.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Worked => "worked",
			Self::Failed => "failed",
			Self::Stopped => "stopped",
		}
	}
}

/// One obligation the app ran to an ending (CK2): a take, a listening, a
/// spoken line, a note sent home.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Outcome {
	/// The phone's id for the event: what makes a retried send store nothing.
	pub id: String,
	/// When the obligation started, as RFC 3339 UTC.
	pub at: String,
	/// The build that ran it (its commit), when the build knows it.
	#[serde(default)]
	pub build: Option<String>,
	/// The process family, by the registry's id (`phone-recording`).
	pub process: String,
	/// The screen it came from, by the code's name for it (`jobs.pass`).
	pub screen: String,
	/// How it ended, from the process's closed union (`kept`, `blocked-denied`).
	pub ending: String,
	pub went: Went,
	/// From the ask to the microphone (or recognizer) being ready.
	#[serde(default)]
	pub ready_ms: Option<u32>,
	/// How long it ran once ready: a take's length.
	#[serde(default)]
	pub duration_ms: Option<u32>,
	/// The loudest measured level, 0 to 1, for a take.
	#[serde(default)]
	pub peak: Option<f64>,
}

/// An app event: its family, and the shape that family has.
///
/// A family is a variant here and nowhere else: the table has no column per
/// family (paulgsc/some-ui ENG2, #2535, adds the second).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum Event {
	Outcome(Outcome),
}

impl Event {
	/// The wire name of the family, which the `family` column holds.
	#[must_use]
	pub const fn family(&self) -> &'static str {
		match self {
			Self::Outcome(_) => "outcome",
		}
	}

	/// The phone's id for it.
	#[must_use]
	pub fn id(&self) -> &str {
		match self {
			Self::Outcome(outcome) => &outcome.id,
		}
	}

	/// When it started, as sent.
	#[must_use]
	pub fn at(&self) -> &str {
		match self {
			Self::Outcome(outcome) => &outcome.at,
		}
	}
}

/// The phone's own ids (`crypto.randomUUID()` today): URL-safe, short.
fn is_id(text: &str) -> bool {
	!text.is_empty() && text.len() <= MAX_ID_CHARS && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// A name from the app's code: lowercase letters, digits, `-` and `.`, at
/// most 64. Room for `jobs.pass` or `blocked-denied`, none for a sentence.
fn is_name(text: &str) -> bool {
	!text.is_empty() && text.len() <= MAX_ID_CHARS && text.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.'))
}

/// `at` as ISO-8601 UTC to the millisecond: the column a window reads by.
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

fn outcome_problems(outcome: &Outcome, problems: &mut Vec<Problem>) {
	if outcome.build.as_deref().is_some_and(|build| !is_id(build)) {
		problems.push(("build", "must be the build's commit: 1 to 64 letters, digits, `-` or `_`, or none"));
	}
	for (field, value) in [("process", &outcome.process), ("screen", &outcome.screen), ("ending", &outcome.ending)] {
		if !is_name(value) {
			problems.push((field, "must be a name from the app's code: 1 to 64 lowercase letters, digits, `-` or `.`"));
		}
	}
	for (field, value) in [("readyMs", outcome.ready_ms), ("durationMs", outcome.duration_ms)] {
		if value.is_some_and(|ms| ms > MAX_MS) {
			problems.push((field, "must be at most a day in milliseconds, or none"));
		}
	}
	if outcome.peak.is_some_and(|peak| !(0.0..=1.0).contains(&peak)) {
		problems.push(("peak", "must be a level from 0 to 1, or none"));
	}
}

/// The event in `body`, checked.
///
/// It must be the one `id` names, and hold every rule above.
///
/// # Errors
/// Every problem found, as `(field, why)`.
pub fn parse_event(id: &str, body: &[u8]) -> Result<Event, Vec<Problem>> {
	if body.len() > BODY_CEILING {
		return Err(vec![("body", "is over the size ceiling")]);
	}
	// Straight from the bytes, so a key written twice is refused rather than
	// read as its last copy.
	let event: Event = serde_json::from_slice(body).map_err(|_| vec![("body", "is not the shape an app event has")])?;
	let mut problems = Vec::new();
	if !is_id(id) {
		problems.push(("id", "must be the phone's id: 1 to 64 letters, digits, `-` or `_`"));
	} else if event.id() != id {
		problems.push(("id", "must be the id the path names"));
	}
	if instant(event.at()).is_none() {
		problems.push(("at", "must be an RFC 3339 instant in UTC, ending `Z`"));
	}
	match &event {
		Event::Outcome(outcome) => outcome_problems(outcome, &mut problems),
	}
	if problems.is_empty() {
		Ok(event)
	} else {
		Err(problems)
	}
}

#[cfg(test)]
mod tests {
	use super::{instant, parse_event, BODY_CEILING, MAX_MS};
	use serde_json::{json, Value};

	fn outcome() -> Value {
		json!({
			"family": "outcome",
			"id": "e1",
			"at": "2026-11-03T09:15:00.000Z",
			"build": "42a72b958",
			"process": "phone-recording",
			"screen": "jobs.pass",
			"ending": "kept",
			"went": "worked",
			"readyMs": 412,
			"durationMs": 5_231,
			"peak": 0.62
		})
	}

	fn fields(id: &str, value: &Value) -> Vec<&'static str> {
		let mut fields: Vec<&'static str> = parse_event(id, value.to_string().as_bytes()).unwrap_err().iter().map(|(field, _)| *field).collect();
		fields.sort_unstable();
		fields
	}

	#[test]
	fn an_outcome_reads_and_writes_back_as_it_came() {
		for went in ["worked", "failed", "stopped"] {
			let mut sent = outcome();
			sent["went"] = json!(went);
			let parsed = parse_event("e1", sent.to_string().as_bytes()).unwrap_or_else(|problems| panic!("{sent}: {problems:?}"));
			assert_eq!(serde_json::to_value(&parsed).unwrap(), sent);
			assert_eq!(parsed.family(), "outcome");
		}
		let mut bare = outcome();
		for key in ["build", "readyMs", "durationMs", "peak"] {
			bare.as_object_mut().unwrap().remove(key);
		}
		let mut nulls = bare.clone();
		for key in ["build", "readyMs", "durationMs", "peak"] {
			nulls[key] = Value::Null;
		}
		assert_eq!(
			serde_json::to_value(parse_event("e1", bare.to_string().as_bytes()).unwrap()).unwrap(),
			nulls,
			"an optional field left out reads back as null"
		);
	}

	#[test]
	fn every_rule_names_its_field() {
		let mut wrong = outcome();
		wrong["at"] = json!("2026-11-03T09:15:00+01:00");
		wrong["build"] = json!("a b");
		wrong["process"] = json!("Phone recording");
		wrong["screen"] = json!("");
		wrong["ending"] = json!("kept, I think");
		wrong["readyMs"] = json!(MAX_MS + 1);
		wrong["durationMs"] = json!(MAX_MS + 1);
		wrong["peak"] = json!(1.5);
		assert_eq!(fields("e1", &wrong), ["at", "build", "durationMs", "ending", "peak", "process", "readyMs", "screen"]);
		assert_eq!(fields("e2", &outcome()), ["id"], "the id the path names");
		assert_eq!(fields("a/b", &outcome()), ["id"]);
	}

	/// C3: nothing outside the family's own fields, and no family the
	/// server does not know, so free text has nowhere to go.
	#[test]
	fn an_unknown_field_or_family_is_off_shape() {
		let mut said = outcome();
		said["words"] = json!("the quick brown fox");
		assert_eq!(fields("e1", &said), ["body"]);
		let mut other = outcome();
		other["family"] = json!("diary");
		assert_eq!(fields("e1", &other), ["body"]);
		let mut negative = outcome();
		negative["readyMs"] = json!(-1);
		assert_eq!(fields("e1", &negative), ["body"]);
		assert_eq!(fields("e1", &json!("outcome")), ["body"]);
		let twice = outcome().to_string().replacen("\"went\":\"worked\"", "\"went\":\"worked\",\"went\":\"failed\"", 1);
		assert!(parse_event("e1", twice.as_bytes()).is_err(), "a key written twice is refused");
		let big = json!({ "family": "outcome", "id": "e1", "pad": "x".repeat(BODY_CEILING) });
		assert_eq!(fields("e1", &big), ["body"]);
	}

	#[test]
	fn instants_are_utc_to_the_millisecond() {
		assert_eq!(instant("2026-11-03T09:15:00Z").as_deref(), Some("2026-11-03T09:15:00.000Z"));
		assert_eq!(instant("2026-11-03T09:15:00+00:00"), None);
	}
}
