//! What a Jobs event is, and the checks a write passes before it is stored.
//! Pure: no database.
//!
//! The shape is the phone's `JobEvent` (`apps/www/src/lib/jobs/model.ts` in
//! paulgsc/some-ui), field for field and in its camelCase, and the closed
//! lists are `@some-ui/job-desk`'s (`PASS_REASONS`, `FRICTION`, `OUTCOMES`).
//! An event names its voice notes by the phone's ids for them: the audio
//! never comes here.

use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

/// `(field, why)`: one reason a write was refused.
pub type Problem = (&'static str, &'static str);

/// The most bytes one event's body may hold. A scout carries the posting's
/// text, up to 20,000 characters; every other event is a few hundred bytes.
pub const EVENT_BODY_CEILING: usize = 64 * 1024;

const MAX_ID_CHARS: usize = 64;
const MAX_NOTE_CHARS: usize = 2_000;
const MAX_FINGERPRINT_CHARS: usize = 400;
const MAX_VOICE: usize = 20;
const MAX_SHARED_VOICE: usize = 200;
const MAX_FILE_CHARS: usize = 128;
const MAX_URL_CHARS: usize = 2_048;
const MAX_TITLE_CHARS: usize = 300;
const MAX_POSTING_CHARS: usize = 20_000;

/// Why a pick was passed (`PASS_REASONS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PassReason {
	Role,
	CustomerFacing,
	Seniority,
	Pay,
	Location,
	Company,
	Closed,
	Other,
}

/// What got in the way of applying (`FRICTION`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Friction {
	AccountWall,
	CompanyAts,
	RetypedResume,
	Assessment,
	CoverLetter,
	LongQuestionnaire,
	FormBroke,
}

/// What came of an application later (`OUTCOMES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
	Viewed,
	Reply,
	Screen,
	Interview,
	Offer,
	Rejected,
	#[serde(rename = "no-reply-30d")]
	NoReply30d,
}

/// How applying ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DebriefResult {
	Submitted,
	GaveUp,
	Closed,
}

/// Which share sheet a `shared` event went out through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShareKind {
	Decisions,
	Scouts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Track {
	Software,
	Adjacent,
}

/// The adjacent track's role families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Family {
	Data,
	Qa,
	Solutions,
}

/// A posting found by hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scout {
	pub id: String,
	pub url: String,
	pub title: String,
	pub why: String,
	pub posting_text: Option<String>,
	pub track: Track,
	/// The adjacent family, or `None` on the software track.
	pub family: Option<Family>,
}

/// One line of the phone's Jobs log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Event {
	#[serde(rename_all = "camelCase")]
	Taken {
		id: String,
		at: String,
		day: String,
		pick_id: String,
		fingerprint: String,
	},
	#[serde(rename_all = "camelCase")]
	Passed {
		id: String,
		at: String,
		day: String,
		pick_id: String,
		fingerprint: String,
		reasons: Vec<PassReason>,
		note: String,
		voice: Vec<String>,
	},
	#[serde(rename_all = "camelCase")]
	ResumeReady {
		id: String,
		at: String,
		day: String,
		pick_id: String,
		fingerprint: String,
	},
	#[serde(rename_all = "camelCase")]
	ApplyOpened {
		id: String,
		at: String,
		day: String,
		pick_id: String,
		fingerprint: String,
	},
	#[serde(rename_all = "camelCase")]
	Debrief {
		id: String,
		at: String,
		day: String,
		pick_id: String,
		fingerprint: String,
		minutes: i64,
		friction: i64,
		chips: Vec<Friction>,
		what: String,
		voice: Vec<String>,
		result: DebriefResult,
	},
	Outcome {
		id: String,
		at: String,
		fingerprint: String,
		event: Outcome,
		on: String,
		note: String,
	},
	Disagree {
		id: String,
		at: String,
		day: String,
		fingerprint: String,
		note: String,
	},
	Scout {
		id: String,
		at: String,
		scout: Scout,
		voice: Vec<String>,
	},
	Shared {
		id: String,
		at: String,
		kind: ShareKind,
		day: String,
		file: String,
		through: i64,
		voice: Vec<String>,
	},
	#[serde(rename_all = "camelCase")]
	Arrived {
		id: String,
		at: String,
		share_id: String,
	},
}

impl Event {
	/// The phone's id for it.
	#[must_use]
	pub fn id(&self) -> &str {
		match self {
			Self::Taken { id, .. }
			| Self::Passed { id, .. }
			| Self::ResumeReady { id, .. }
			| Self::ApplyOpened { id, .. }
			| Self::Debrief { id, .. }
			| Self::Outcome { id, .. }
			| Self::Disagree { id, .. }
			| Self::Scout { id, .. }
			| Self::Shared { id, .. }
			| Self::Arrived { id, .. } => id,
		}
	}

	/// When the phone recorded it, as written.
	#[must_use]
	pub fn at(&self) -> &str {
		match self {
			Self::Taken { at, .. }
			| Self::Passed { at, .. }
			| Self::ResumeReady { at, .. }
			| Self::ApplyOpened { at, .. }
			| Self::Debrief { at, .. }
			| Self::Outcome { at, .. }
			| Self::Disagree { at, .. }
			| Self::Scout { at, .. }
			| Self::Shared { at, .. }
			| Self::Arrived { at, .. } => at,
		}
	}
}

/// The phone's own ids (`crypto.randomUUID()` today): URL-safe, short.
fn is_event_id(text: &str) -> bool {
	!text.is_empty() && text.len() <= MAX_ID_CHARS && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// A digest's pick and scout ids (`IdSchema` in `@some-ui/job-desk`):
/// lowercase letters, digits and dashes, 3 to 41 of them, not starting with
/// a dash.
fn is_pick_id(text: &str) -> bool {
	let bytes = text.as_bytes();
	(3..=41).contains(&bytes.len()) && bytes[0] != b'-' && bytes.iter().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

/// `company|title|location` (`fingerprint()` in `@some-ui/job-desk`): three
/// non-empty parts.
fn is_fingerprint(text: &str) -> bool {
	!too_long(text, MAX_FINGERPRINT_CHARS) && text.split('|').count() == 3 && text.split('|').all(|part| !part.is_empty())
}

/// A calendar day that exists, written `YYYY-MM-DD`.
fn is_day(text: &str) -> bool {
	text.len() == 10 && NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok()
}

/// A shared file's name: `decisions-<day>.json`, `scouts-<stamp>.json`.
fn is_file_name(text: &str) -> bool {
	!too_long(text, MAX_FILE_CHARS)
		&& std::path::Path::new(text).extension().is_some_and(|extension| extension == "json")
		&& text.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn too_long(text: &str, chars: usize) -> bool {
	text.chars().count() > chars
}

/// `at` as ISO-8601 UTC to the millisecond, or `None` when it is not
/// RFC 3339: the column a read orders by.
#[must_use]
pub fn instant(at: &str) -> Option<String> {
	DateTime::parse_from_rfc3339(at)
		.ok()
		.map(|at| at.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn has_duplicates<T: PartialEq>(items: &[T]) -> bool {
	items.iter().enumerate().any(|(i, item)| items[..i].contains(item))
}

/// Problems with a pick reference: the digest's day, the pick's id in it,
/// and the posting's fingerprint.
fn pick_ref(problems: &mut Vec<Problem>, day: &str, pick_id: &str, fingerprint: &str) {
	if !is_day(day) {
		problems.push(("day", "must be a calendar day, YYYY-MM-DD"));
	}
	if !is_pick_id(pick_id) {
		problems.push(("pickId", "must be a digest's pick id: 3 to 41 lowercase letters, digits and dashes"));
	}
	fingerprint_ok(problems, fingerprint);
}

fn fingerprint_ok(problems: &mut Vec<Problem>, fingerprint: &str) {
	if !is_fingerprint(fingerprint) {
		problems.push(("fingerprint", "must be company|title|location, at most 400 characters"));
	}
}

fn note_ok(problems: &mut Vec<Problem>, field: &'static str, note: &str) {
	if too_long(note, MAX_NOTE_CHARS) {
		problems.push((field, "is at most 2000 characters"));
	}
}

fn voice_ok(problems: &mut Vec<Problem>, voice: &[String], most: usize) {
	if voice.len() > most || voice.iter().any(|id| !is_event_id(id)) || has_duplicates(voice) {
		problems.push(("voice", "names each voice note once, by the phone's id for it, and only so many"));
	}
}

fn scout_ok(problems: &mut Vec<Problem>, scout: &Scout) {
	if !is_pick_id(&scout.id) {
		problems.push(("scout.id", "must be 3 to 41 lowercase letters, digits and dashes"));
	}
	if !scout.url.starts_with("https://") || too_long(&scout.url, MAX_URL_CHARS) {
		problems.push(("scout.url", "must be an https link, at most 2048 characters"));
	}
	if scout.title.trim().is_empty() || too_long(&scout.title, MAX_TITLE_CHARS) {
		problems.push(("scout.title", "is 1 to 300 characters"));
	}
	note_ok(problems, "scout.why", &scout.why);
	if scout.posting_text.as_deref().is_some_and(|text| too_long(text, MAX_POSTING_CHARS)) {
		problems.push(("scout.postingText", "is at most 20000 characters"));
	}
	if (scout.track == Track::Adjacent) != scout.family.is_some() {
		problems.push(("scout.family", "an adjacent scout names its family; a software scout has none"));
	}
}

/// The problems with one event's payload, by its type.
fn payload(event: &Event, problems: &mut Vec<Problem>) {
	match event {
		Event::Taken { day, pick_id, fingerprint, .. } | Event::ResumeReady { day, pick_id, fingerprint, .. } | Event::ApplyOpened { day, pick_id, fingerprint, .. } => {
			pick_ref(problems, day, pick_id, fingerprint);
		}
		Event::Passed {
			day,
			pick_id,
			fingerprint,
			reasons,
			note,
			voice,
			..
		} => {
			pick_ref(problems, day, pick_id, fingerprint);
			if has_duplicates(reasons) {
				problems.push(("reasons", "names each reason once"));
			}
			note_ok(problems, "note", note);
			voice_ok(problems, voice, MAX_VOICE);
		}
		Event::Debrief {
			day,
			pick_id,
			fingerprint,
			minutes,
			friction,
			chips,
			what,
			voice,
			..
		} => {
			pick_ref(problems, day, pick_id, fingerprint);
			if !(1..=600).contains(minutes) {
				problems.push(("minutes", "is a whole number of minutes, 1 to 600"));
			}
			if !(1..=5).contains(friction) {
				problems.push(("friction", "is 1 to 5"));
			}
			if has_duplicates(chips) {
				problems.push(("chips", "names each chip once"));
			}
			note_ok(problems, "what", what);
			voice_ok(problems, voice, MAX_VOICE);
		}
		Event::Outcome { fingerprint, on, note, .. } => {
			fingerprint_ok(problems, fingerprint);
			if !is_day(on) {
				problems.push(("on", "must be a calendar day, YYYY-MM-DD"));
			}
			note_ok(problems, "note", note);
		}
		Event::Disagree { day, fingerprint, note, .. } => {
			if !is_day(day) {
				problems.push(("day", "must be a calendar day, YYYY-MM-DD"));
			}
			fingerprint_ok(problems, fingerprint);
			note_ok(problems, "note", note);
		}
		Event::Scout { scout, voice, .. } => {
			scout_ok(problems, scout);
			voice_ok(problems, voice, MAX_VOICE);
		}
		Event::Shared { day, file, through, voice, .. } => {
			if !is_day(day) {
				problems.push(("day", "must be a calendar day, YYYY-MM-DD"));
			}
			if !is_file_name(file) {
				problems.push(("file", "must be a .json file name, at most 128 characters"));
			}
			if *through < 0 {
				problems.push(("through", "is a count of events, 0 or more"));
			}
			voice_ok(problems, voice, MAX_SHARED_VOICE);
		}
		Event::Arrived { share_id, .. } => {
			if !is_event_id(share_id) {
				problems.push(("shareId", "must be the phone's id for a share"));
			}
		}
	}
}

/// The event in `body`, checked.
///
/// It must be the one `id` names, and hold every rule its type holds.
///
/// # Errors
/// Every problem found, as `(field, why)`.
pub fn parse_event(id: &str, body: &[u8]) -> Result<Event, Vec<Problem>> {
	if body.len() > EVENT_BODY_CEILING {
		return Err(vec![("body", "is over the size ceiling")]);
	}
	let event: Event = serde_json::from_slice(body).map_err(|_| vec![("body", "is not the shape this record holds")])?;
	let mut problems = Vec::new();
	if !is_event_id(id) {
		problems.push(("id", "must be the phone's id for the event: 1 to 64 letters, digits, `-` or `_`"));
	} else if event.id() != id {
		problems.push(("id", "must be the id the path names"));
	}
	if instant(event.at()).is_none() {
		problems.push(("at", "must be an RFC 3339 instant"));
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

	fn fields(id: &str, value: &Value) -> Vec<&'static str> {
		let mut fields: Vec<&str> = parse_event(id, value.to_string().as_bytes()).unwrap_err().iter().map(|(field, _)| *field).collect();
		fields.sort_unstable();
		fields
	}

	fn debrief() -> Value {
		json!({
			"id": "e1", "at": "2026-10-18T22:10:00.000Z", "type": "debrief",
			"day": "2026-10-18", "pickId": "blu-7f2a", "fingerprint": "bluesight|data analyst|remote",
			"minutes": 23, "friction": 3, "chips": ["company-ats"], "what": "Indeed sent me to Workday.",
			"voice": ["0b6f0f8e-1f2a-4c55-9d7e-3a1b2c3d4e5f"], "result": "submitted"
		})
	}

	#[test]
	fn every_phone_event_type_reads_and_writes_back_as_it_came() {
		let pick = json!({ "day": "2026-10-18", "pickId": "blu-7f2a", "fingerprint": "bluesight|data analyst|remote" });
		let with = |kind: &str, extra: Value| {
			let mut event = json!({ "id": "e1", "at": "2026-10-18T22:10:00.000Z", "type": kind });
			for (key, value) in extra.as_object().unwrap().iter().chain(pick.as_object().unwrap()) {
				event[key] = value.clone();
			}
			event
		};
		let events = [
			with("taken", json!({})),
			with("passed", json!({ "reasons": ["seniority", "customer-facing"], "note": "Clearance", "voice": [] })),
			with("resume-ready", json!({})),
			with("apply-opened", json!({})),
			debrief(),
			json!({ "id": "e1", "at": "2026-10-18T22:10:00.000Z", "type": "outcome", "fingerprint": "a|b|c", "event": "no-reply-30d", "on": "2026-11-17", "note": "" }),
			json!({ "id": "e1", "at": "2026-10-18T22:10:00.000Z", "type": "disagree", "day": "2026-10-18", "fingerprint": "a|b|c", "note": "I'd take it" }),
			json!({ "id": "e1", "at": "2026-10-18T22:10:00.000Z", "type": "scout", "voice": [], "scout": {
				"id": "sc-a1b2c3d4", "url": "https://www.indeed.com/viewjob?jk=0123", "title": "SDET · Second Dinner",
				"why": "Game testing", "postingText": null, "track": "adjacent", "family": "qa"
			} }),
			json!({ "id": "e1", "at": "2026-10-18T22:10:00.000Z", "type": "shared", "kind": "decisions", "day": "2026-10-18", "file": "decisions-2026-10-18.json", "through": 7, "voice": [] }),
			json!({ "id": "e1", "at": "2026-10-18T22:10:00.000Z", "type": "arrived", "shareId": "e0" }),
		];
		for event in events {
			let parsed = parse_event("e1", event.to_string().as_bytes()).unwrap_or_else(|problems| panic!("{event}: {problems:?}"));
			assert_eq!(serde_json::to_value(&parsed).unwrap(), event, "{event}");
		}
	}

	#[test]
	fn a_body_off_the_shape_is_refused_whole() {
		let mut extra = debrief();
		extra["mood"] = json!("tired");
		assert_eq!(fields("e1", &extra), ["body"], "no field the phone does not send");
		let mut unknown = debrief();
		unknown["type"] = json!("deleted");
		assert_eq!(fields("e1", &unknown), ["body"], "no type the phone does not record");
		let mut chip = debrief();
		chip["chips"] = json!(["recruiter-call"]);
		assert_eq!(fields("e1", &chip), ["body"], "chips come from the phone's list");
		let mut half = debrief();
		half["minutes"] = json!(12.5);
		assert_eq!(fields("e1", &half), ["body"], "minutes are whole");
		let big = json!({ "id": "e1", "padding": "x".repeat(super::EVENT_BODY_CEILING) });
		assert_eq!(fields("e1", &big), ["body"]);
	}

	#[test]
	fn each_field_is_held_to_the_phones_rules() {
		let mut wrong = debrief();
		wrong["at"] = json!("yesterday");
		wrong["day"] = json!("2026-02-30");
		wrong["pickId"] = json!("Blu_7f2a");
		wrong["fingerprint"] = json!("bluesight|data analyst");
		wrong["minutes"] = json!(0);
		wrong["friction"] = json!(6);
		wrong["chips"] = json!(["assessment", "assessment"]);
		wrong["what"] = json!("x".repeat(2_001));
		wrong["voice"] = json!(["../audio.webm"]);
		assert_eq!(
			fields("e1", &wrong),
			["at", "chips", "day", "fingerprint", "friction", "minutes", "pickId", "voice", "what"]
		);
		assert_eq!(fields("e2", &debrief()), ["id"], "the id the path names");
		assert_eq!(fields("e/1", &debrief()), ["id"], "a plain id");

		let scout = |track: &str, family: Value, url: &str| {
			json!({ "id": "e1", "at": "2026-10-18T22:10:00Z", "type": "scout", "voice": [], "scout": {
				"id": "SC-1", "url": url, "title": " ", "why": "", "postingText": "x".repeat(20_001),
				"track": track, "family": family
			} })
		};
		assert_eq!(
			fields("e1", &scout("software", json!("qa"), "http://indeed.com/x")),
			["scout.family", "scout.id", "scout.postingText", "scout.title", "scout.url"]
		);
		let shared =
			json!({ "id": "e1", "at": "2026-10-18T22:10:00Z", "type": "shared", "kind": "scouts", "day": "2026-10-18", "file": "../scouts.txt", "through": -1, "voice": [] });
		assert_eq!(fields("e1", &shared), ["file", "through"]);
	}

	#[test]
	fn the_order_a_read_serves_is_utc_to_the_millisecond() {
		assert_eq!(instant("2026-10-18T15:10:00-07:00").as_deref(), Some("2026-10-18T22:10:00.000Z"));
		assert_eq!(instant("2026-10-18T22:10:00.5Z").as_deref(), Some("2026-10-18T22:10:00.500Z"));
		assert_eq!(instant("2026-10-18"), None);
		let event: Event = parse_event("e1", debrief().to_string().as_bytes()).unwrap();
		assert_eq!((event.id(), event.at()), ("e1", "2026-10-18T22:10:00.000Z"));
	}
}
