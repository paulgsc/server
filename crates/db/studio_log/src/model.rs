//! What a Studio sitting event and a friction report are in their home form,
//! and the checks a write passes before it is stored. Pure: no database.
//!
//! The wire form is the one paulgsc/some-ui ST9 (#2257) sends: plain JSON
//! objects, every key one word, every closed value `snake_case`. An optional
//! field the phone leaves out reads back as `null`, as one it sends as `null`
//! does, so a resend either way compares equal (Jobs' `minutes` and
//! `friction` do the same).

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

/// `(field, why)`: one reason a write was refused.
pub type Problem = (&'static str, &'static str);

/// The most bytes one event's or report's body may hold.
///
/// Either is a few hundred bytes: ids, a screen or shot name, a cause of at
/// most [`MAX_CAUSE_CHARS`] characters. Every field is bounded on its own, so
/// this only stops a body from being read whole before it is refused.
pub const BODY_CEILING: usize = 16 * 1024;

/// The most characters a friction report's `cause` may hold.
pub const MAX_CAUSE_CHARS: usize = 200;

const MAX_ID_CHARS: usize = 64;
const MAX_NAME_CHARS: usize = 200;

/// The kind of sitting: what the owner sat down to do to the video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
	Record,
	Watch,
	Shape,
	Finish,
}

impl Kind {
	/// The wire name, which the `kind` column holds.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Record => "record",
			Self::Watch => "watch",
			Self::Shape => "shape",
			Self::Finish => "finish",
		}
	}
}

/// What a sitting event says happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Happened {
	SittingOpened,
	SittingClosed,
	ScreenEntered,
	ScreenLeft,
	PageHidden,
	PageShown,
}

impl Happened {
	/// The wire name, which the `type` column holds.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::SittingOpened => "sitting_opened",
			Self::SittingClosed => "sitting_closed",
			Self::ScreenEntered => "screen_entered",
			Self::ScreenLeft => "screen_left",
			Self::PageHidden => "page_hidden",
			Self::PageShown => "page_shown",
		}
	}

	/// Whether the event is about one screen, and so must name it.
	const fn names_a_screen(self) -> bool {
		matches!(self, Self::ScreenEntered | Self::ScreenLeft)
	}
}

/// One line of a sitting's log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
	/// The phone's id for the event: what makes a retried send store nothing.
	pub id: String,
	/// When the phone recorded it, as RFC 3339 UTC.
	pub at: String,
	/// The phone's id for the sitting.
	pub sitting: String,
	pub kind: Kind,
	#[serde(rename = "type")]
	pub happened: Happened,
	/// The screen, by the phone's name for it: required when the event is a
	/// screen entered or left, optional otherwise.
	#[serde(default)]
	pub screen: Option<String>,
}

/// How a sitting went, as the owner said at its end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum How {
	Smooth,
	SomeSnags,
	Rough,
}

impl How {
	/// The wire name, which the `how` column holds.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Smooth => "smooth",
			Self::SomeSnags => "some_snags",
			Self::Rough => "rough",
		}
	}
}

/// A friction report: how one sitting went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
	/// The phone's id for the report: what makes a retried send store nothing.
	pub id: String,
	/// When the phone recorded it, as RFC 3339 UTC.
	pub at: String,
	/// The sitting it reports on.
	pub sitting: String,
	pub kind: Kind,
	pub how: How,
	/// The cause the owner chose or said, in at most [`MAX_CAUSE_CHARS`]
	/// characters. It may be typed by the owner, so it never reaches a log.
	#[serde(default)]
	pub cause: Option<String>,
	/// The shot the sitting was on, by the phone's id for it.
	#[serde(default)]
	pub shot: Option<String>,
	/// A voice note left with the report, by the phone's id for it. Its
	/// audio, once sent, is `voice_note_audio`'s.
	#[serde(default)]
	pub voice: Option<String>,
}

/// The phone's own ids (`crypto.randomUUID()` today): URL-safe, short.
fn is_id(text: &str) -> bool {
	!text.is_empty() && text.len() <= MAX_ID_CHARS && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// A screen's or a shot's name, as the phone's code names it: the rule a
/// voice note's `ref` keeps.
fn is_name(text: &str) -> bool {
	!text.is_empty() && text.len() <= MAX_NAME_CHARS && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn is_cause(text: &str) -> bool {
	!text.trim().is_empty() && text.chars().count() <= MAX_CAUSE_CHARS && !text.chars().any(char::is_control)
}

/// `at` as ISO-8601 UTC to the millisecond: the column a read orders by.
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

const OFF_SHAPE: Problem = ("body", "is not the shape this record holds");

fn read<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, Vec<Problem>> {
	if body.len() > BODY_CEILING {
		return Err(vec![("body", "is over the size ceiling")]);
	}
	// Straight from the bytes, so a key written twice is refused rather than
	// read as its last copy.
	serde_json::from_slice::<T>(body).map_err(|_| vec![OFF_SHAPE])
}

fn ids(problems: &mut Vec<Problem>, id: &str, path_id: &str, at: &str) {
	if !is_id(path_id) {
		problems.push(("id", "must be the phone's id: 1 to 64 letters, digits, `-` or `_`"));
	} else if id != path_id {
		problems.push(("id", "must be the id the path names"));
	}
	if instant(at).is_none() {
		problems.push(("at", "must be an RFC 3339 instant in UTC, ending `Z`"));
	}
}

fn done<T>(value: T, problems: Vec<Problem>) -> Result<T, Vec<Problem>> {
	if problems.is_empty() {
		Ok(value)
	} else {
		Err(problems)
	}
}

/// The sitting event in `body`, checked.
///
/// It must be the one `sitting` and `id` name, and hold every rule above.
///
/// # Errors
/// Every problem found, as `(field, why)`.
pub fn parse_event(sitting: &str, id: &str, body: &[u8]) -> Result<Event, Vec<Problem>> {
	let event: Event = read(body)?;
	let mut problems = Vec::new();
	ids(&mut problems, &event.id, id, &event.at);
	if !is_id(sitting) {
		problems.push(("sitting", "must be the phone's id for the sitting: 1 to 64 letters, digits, `-` or `_`"));
	} else if event.sitting != sitting {
		problems.push(("sitting", "must be the sitting the path names"));
	}
	match &event.screen {
		Some(screen) if !is_name(screen) => problems.push(("screen", "must be the screen's name: 1 to 200 letters, digits, '-', '_', '.' or ':'")),
		None if event.happened.names_a_screen() => problems.push(("screen", "must name the screen entered or left")),
		Some(_) | None => {}
	}
	done(event, problems)
}

/// The friction report in `body`, checked.
///
/// It must be the one `id` names, and hold every rule above.
///
/// # Errors
/// Every problem found, as `(field, why)`.
pub fn parse_report(id: &str, body: &[u8]) -> Result<Report, Vec<Problem>> {
	let report: Report = read(body)?;
	let mut problems = Vec::new();
	ids(&mut problems, &report.id, id, &report.at);
	if !is_id(&report.sitting) {
		problems.push(("sitting", "must be the phone's id for the sitting: 1 to 64 letters, digits, `-` or `_`"));
	}
	if report.cause.as_deref().is_some_and(|cause| !is_cause(cause)) {
		problems.push(("cause", "is 1 to 200 characters, not blank, no control characters, or unsaid"));
	}
	if report.shot.as_deref().is_some_and(|shot| !is_name(shot)) {
		problems.push(("shot", "must be the shot's id: 1 to 200 letters, digits, '-', '_', '.' or ':', or none"));
	}
	if report.voice.as_deref().is_some_and(|voice| !is_id(voice)) {
		problems.push(("voice", "must be the phone's id for the voice note: 1 to 64 letters, digits, `-` or `_`, or none"));
	}
	done(report, problems)
}

#[cfg(test)]
mod tests {
	use super::{instant, parse_event, parse_report, BODY_CEILING, MAX_CAUSE_CHARS};
	use serde_json::{json, Value};

	const S: &str = "4f0c2a8e-7b1d-4c3e-9a55-0d1e2f3a4b5c";

	fn sorted(mut fields: Vec<&'static str>) -> Vec<&'static str> {
		fields.sort_unstable();
		fields
	}

	fn event_fields(sitting: &str, id: &str, value: &Value) -> Vec<&'static str> {
		sorted(
			parse_event(sitting, id, value.to_string().as_bytes())
				.unwrap_err()
				.iter()
				.map(|(field, _)| *field)
				.collect(),
		)
	}

	fn report_fields(id: &str, value: &Value) -> Vec<&'static str> {
		sorted(parse_report(id, value.to_string().as_bytes()).unwrap_err().iter().map(|(field, _)| *field).collect())
	}

	fn event(kind: &str, happened: &str, screen: &Value) -> Value {
		json!({ "id": "e1", "at": "2026-10-30T09:15:00.000Z", "sitting": S, "kind": kind, "type": happened, "screen": screen })
	}

	fn report(how: &str) -> Value {
		json!({ "id": "f1", "at": "2026-10-30T10:00:00.000Z", "sitting": S, "kind": "record", "how": how, "cause": null, "shot": null, "voice": null })
	}

	#[test]
	fn every_event_reads_and_writes_back_as_it_came() {
		for kind in ["record", "watch", "shape", "finish"] {
			for happened in ["sitting_opened", "sitting_closed", "screen_entered", "screen_left", "page_hidden", "page_shown"] {
				let sent = event(kind, happened, &json!("record.shot-list"));
				let parsed = parse_event(S, "e1", sent.to_string().as_bytes()).unwrap_or_else(|problems| panic!("{sent}: {problems:?}"));
				assert_eq!(serde_json::to_value(&parsed).unwrap(), sent);
				assert_eq!((parsed.kind.as_str(), parsed.happened.as_str()), (kind, happened), "the columns are the body's");
			}
		}
		let opened = event("record", "sitting_opened", &Value::Null);
		assert_eq!(serde_json::to_value(parse_event(S, "e1", opened.to_string().as_bytes()).unwrap()).unwrap(), opened);
		let mut left_out = opened.clone();
		left_out.as_object_mut().unwrap().remove("screen");
		assert_eq!(
			serde_json::to_value(parse_event(S, "e1", left_out.to_string().as_bytes()).unwrap()).unwrap(),
			opened,
			"a screen left out reads back as null"
		);
	}

	#[test]
	fn every_report_reads_and_writes_back_as_it_came() {
		for how in ["smooth", "some_snags", "rough"] {
			let sent = report(how);
			let parsed = parse_report("f1", sent.to_string().as_bytes()).unwrap_or_else(|problems| panic!("{sent}: {problems:?}"));
			assert_eq!(serde_json::to_value(&parsed).unwrap(), sent);
			assert_eq!(parsed.how.as_str(), how);
		}
		let mut full = report("rough");
		full["cause"] = json!("The take kept clipping — mic too close");
		full["shot"] = json!("build-journal-ep04:s3");
		full["voice"] = json!("9b2d1c40-1111-4c3e-9a55-0d1e2f3a4b5c");
		assert_eq!(serde_json::to_value(parse_report("f1", full.to_string().as_bytes()).unwrap()).unwrap(), full);
		let bare = json!({ "id": "f1", "at": "2026-10-30T10:00:00.000Z", "sitting": S, "kind": "record", "how": "smooth" });
		assert_eq!(
			serde_json::to_value(parse_report("f1", bare.to_string().as_bytes()).unwrap()).unwrap(),
			report("smooth"),
			"what is left out reads back as null"
		);
	}

	#[test]
	fn a_body_off_the_shape_is_refused_whole() {
		let mut extra = event("record", "page_hidden", &Value::Null);
		extra["mood"] = json!("tired");
		assert_eq!(event_fields(S, "e1", &extra), ["body"], "no field the phone does not send");
		assert_eq!(
			event_fields(S, "e1", &event("edit", "page_hidden", &Value::Null)),
			["body"],
			"kinds come from the phone's list"
		);
		assert_eq!(event_fields(S, "e1", &event("record", "deleted", &Value::Null)), ["body"], "and what happened");
		let twice = event("record", "page_hidden", &Value::Null)
			.to_string()
			.replacen("\"id\":\"e1\"", "\"id\":\"e1\",\"id\":\"e1\"", 1);
		assert!(parse_event(S, "e1", twice.as_bytes()).is_err(), "a key written twice");
		let big = json!({ "id": "e1", "padding": "x".repeat(BODY_CEILING) });
		assert_eq!(event_fields(S, "e1", &big), ["body"]);
		assert_eq!(event_fields(S, "e1", &json!(["event"])), ["body"]);

		let mut unknown = report("smooth");
		unknown["note"] = json!("a long story");
		assert_eq!(report_fields("f1", &unknown), ["body"], "a report holds no free note");
		assert_eq!(report_fields("f1", &report("bad")), ["body"], "how it went comes from the phone's list");
		let mut missing = report("smooth");
		missing.as_object_mut().unwrap().remove("kind");
		assert_eq!(report_fields("f1", &missing), ["body"]);
	}

	#[test]
	fn each_field_is_held_to_its_rule() {
		let mut wrong = event("record", "screen_entered", &Value::Null);
		wrong["at"] = json!("2026-10-30T02:15:00-07:00");
		assert_eq!(event_fields(S, "e1", &wrong), ["at", "screen"], "the phone writes UTC, and an entered screen is named");
		assert_eq!(event_fields(S, "e1", &event("record", "screen_left", &json!(""))), ["screen"]);
		assert_eq!(
			event_fields(S, "e1", &event("record", "page_shown", &json!("shot list"))),
			["screen"],
			"a name, not a sentence"
		);
		assert_eq!(event_fields(S, "e2", &event("record", "page_shown", &Value::Null)), ["id"], "the id the path names");
		assert_eq!(event_fields(S, "e/1", &event("record", "page_shown", &Value::Null)), ["id"]);
		assert_eq!(
			event_fields("other", "e1", &event("record", "page_shown", &Value::Null)),
			["sitting"],
			"the sitting the path names"
		);
		assert_eq!(event_fields("a:b", "e1", &event("record", "page_shown", &Value::Null)), ["sitting"]);

		let mut bad = report("rough");
		bad["at"] = json!("yesterday");
		bad["sitting"] = json!("a b");
		bad["cause"] = json!("   ");
		bad["shot"] = json!("");
		bad["voice"] = json!("n/1");
		assert_eq!(report_fields("f1", &bad), ["at", "cause", "shot", "sitting", "voice"]);
		assert_eq!(report_fields("f2", &report("rough")), ["id"]);
		let mut long = report("rough");
		long["cause"] = json!("한".repeat(MAX_CAUSE_CHARS));
		assert!(parse_report("f1", long.to_string().as_bytes()).is_ok(), "200 characters, not bytes");
		long["cause"] = json!("x".repeat(MAX_CAUSE_CHARS + 1));
		assert_eq!(report_fields("f1", &long), ["cause"]);
		long["cause"] = json!("one\ntwo");
		assert_eq!(report_fields("f1", &long), ["cause"], "one line");
	}

	#[test]
	fn the_order_a_read_serves_is_utc_to_the_millisecond() {
		assert_eq!(instant("2026-10-30T09:15:00Z").as_deref(), Some("2026-10-30T09:15:00.000Z"));
		assert_eq!(instant("2026-10-30T09:15:00.5Z").as_deref(), Some("2026-10-30T09:15:00.500Z"));
		assert_eq!(instant("2026-10-30T02:15:00-07:00"), None);
		assert_eq!(instant("2026-10-30"), None);
	}
}
