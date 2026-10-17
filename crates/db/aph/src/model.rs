//! What an aph entry and aph's settings are, and the checks a write passes
//! before it is stored. Pure: no database.
//!
//! The shapes are the phone's (`packages/ui/aph/src/lib/model.ts` in
//! paulgsc/some-ui), field for field and in its camelCase. Every figure is a
//! whole number, as the phone's logger only ever writes one.

use serde::{Deserialize, Serialize};

/// `(field, why)`: one reason a write was refused.
pub type Problem = (&'static str, &'static str);

/// The most bytes one entry's body may hold. An entry is a few hundred.
pub const ENTRY_BODY_CEILING: usize = 4 * 1024;

/// The most bytes the settings' body may hold.
pub const SETTINGS_BODY_CEILING: usize = 16 * 1024;

const MAX_LABELS: usize = 200;
const MAX_LABEL_CHARS: usize = 64;
const MAX_NOTE_CHARS: usize = 1_000;
const MAX_TIME_CHARS: usize = 16;
const MAX_CHECKPOINTS: usize = 12;
const MINUTES_IN_A_DAY: i64 = 24 * 60;

/// My figure: what I observed, and whether it was rough ("~4,300").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mine {
	pub value: i64,
	pub approx: bool,
}

/// Their figure, as reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Theirs {
	pub value: i64,
}

/// My call on an entry whose figures do not reconcile by themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Review {
	Agreed,
	Flagged,
}

/// One entry: a day, a time of day, and the figures logged for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Entry {
	pub id: String,
	/// Local calendar day, `YYYY-MM-DD`.
	pub day: String,
	/// The checkpoint it belongs to, or `None` for any other time of day.
	pub checkpoint: Option<String>,
	/// The clock time as written, off the checkpoints only.
	pub time: Option<String>,
	pub mine: Option<Mine>,
	pub theirs: Option<Theirs>,
	/// The goal when the entry was made: at a checkpoint only.
	pub goal: Option<i64>,
	pub labels: Vec<String>,
	pub note: Option<String>,
	pub review: Option<Review>,
}

impl Entry {
	/// Carries no label: the entry that stands for its checkpoint that day.
	#[must_use]
	pub const fn is_plain(&self) -> bool {
		self.labels.is_empty()
	}
}

/// A fixed time of day an entry is taken at, and what aph should be then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
	pub id: String,
	pub label: String,
	/// Minutes after local midnight.
	pub minutes: i64,
	pub goal: i64,
}

/// What the entries are read against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
	pub checkpoints: Vec<Checkpoint>,
	pub labels: Vec<String>,
	pub step: i64,
	pub usual_low: i64,
	pub usual_high: i64,
	pub tolerance: i64,
	/// The first day tracked, `YYYY-MM-DD`.
	pub since: String,
}

impl Settings {
	#[must_use]
	pub fn has_checkpoint(&self, id: &str) -> bool {
		self.checkpoints.iter().any(|checkpoint| checkpoint.id == id)
	}
}

const NOT_A_KEY: Problem = ("id", "must be a plain key: URL-unreserved characters, not starting with `.` or `http`, no `.json` suffix");
const NOT_A_DAY: &str = "must be a calendar day, YYYY-MM-DD";

/// A calendar day that exists, written `YYYY-MM-DD`: four digits, a dash,
/// a month from 01 to 12, a dash, and a day that month has.
fn is_day(text: &str) -> bool {
	let bytes = text.as_bytes();
	let shaped = bytes.len() == 10
		&& bytes
			.iter()
			.enumerate()
			.all(|(i, byte)| if i == 4 || i == 7 { *byte == b'-' } else { byte.is_ascii_digit() });
	if !shaped {
		return false;
	}
	let number = |range: std::ops::Range<usize>| text[range].parse::<u32>().unwrap_or(0);
	let (year, month, day) = (number(0..4), number(5..7), number(8..10));
	let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
	let days = match month {
		1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
		4 | 6 | 9 | 11 => 30,
		2 if leap => 29,
		2 => 28,
		_ => return false,
	};
	(1..=days).contains(&day)
}

fn too_long(text: &str, chars: usize) -> bool {
	text.chars().count() > chars
}

fn labels_problem(labels: &[String]) -> Option<&'static str> {
	if labels.len() > MAX_LABELS {
		return Some("holds more than 200 labels");
	}
	if labels.iter().any(|label| label.trim().is_empty() || too_long(label, MAX_LABEL_CHARS)) {
		return Some("each label is 1 to 64 characters");
	}
	None
}

/// Reads `body` as JSON of the shape `T`, within `ceiling` bytes.
fn read<T: for<'de> Deserialize<'de>>(body: &[u8], ceiling: usize) -> Result<T, Vec<Problem>> {
	if body.len() > ceiling {
		return Err(vec![("body", "is over the size ceiling")]);
	}
	serde_json::from_slice(body).map_err(|_| vec![("body", "is not the shape this record holds")])
}

/// The entry in `body`, checked.
///
/// It must be the one `id` names, and hold every rule an entry holds on its
/// own (`violations` in the phone's model.ts, as far as one entry and the
/// settings can say).
///
/// # Errors
/// Every problem found, as `(field, why)`.
pub fn parse_entry(id: &str, body: &[u8], settings: Option<&Settings>) -> Result<Entry, Vec<Problem>> {
	let entry: Entry = read(body, ENTRY_BODY_CEILING)?;
	let mut problems = Vec::new();
	if !curriculum_repo::is_plain_key(id) {
		problems.push(NOT_A_KEY);
	} else if entry.id != id {
		problems.push(("id", "must be the id the path names"));
	}
	if !is_day(&entry.day) {
		problems.push(("day", NOT_A_DAY));
	}
	if entry.mine.is_none() && entry.theirs.is_none() {
		problems.push(("mine", "an entry holds at least one figure, mine or theirs"));
	}
	match &entry.checkpoint {
		Some(checkpoint) => {
			if entry.goal.is_none() {
				problems.push(("goal", "an entry at a checkpoint carries the goal it was made under"));
			}
			if entry.time.is_some() {
				problems.push(("time", "is written only off the checkpoints"));
			}
			if !settings.is_some_and(|settings| settings.has_checkpoint(checkpoint)) {
				problems.push(("checkpoint", "is not one of the settings' checkpoints"));
			}
		}
		None => {
			if entry.goal.is_some() {
				problems.push(("goal", "an entry off the checkpoints carries none"));
			}
		}
	}
	if entry.time.as_deref().is_some_and(|time| too_long(time, MAX_TIME_CHARS)) {
		problems.push(("time", "is at most 16 characters"));
	}
	if let Some(why) = labels_problem(&entry.labels) {
		problems.push(("labels", why));
	}
	if entry.note.as_deref().is_some_and(|note| too_long(note, MAX_NOTE_CHARS)) {
		problems.push(("note", "is at most 1000 characters"));
	}
	if problems.is_empty() {
		Ok(entry)
	} else {
		Err(problems)
	}
}

/// What a revision of an entry must not change from the one before it.
///
/// As `breaches` in the phone's model.ts: its day, checkpoint, clock time and
/// goal stay as logged, and their figure, once reported, stays.
#[must_use]
pub fn breaches(was: &Entry, is: &Entry) -> Vec<Problem> {
	let mut problems = Vec::new();
	if is.day != was.day {
		problems.push(("day", "is rewritten: an entry stays on the day it was logged"));
	}
	if is.checkpoint != was.checkpoint {
		problems.push(("checkpoint", "is rewritten: an entry stays at the time it was logged"));
	}
	if is.time != was.time {
		problems.push(("time", "is rewritten: an entry stays at the time it was logged"));
	}
	if is.goal != was.goal {
		problems.push(("goal", "is rewritten: an entry keeps the goal it was made under"));
	}
	if was.theirs.is_some() && is.theirs != was.theirs {
		problems.push(("theirs", "is replaced: their figure, once reported, stays"));
	}
	problems
}

/// The settings in `body`, checked.
///
/// # Errors
/// Every problem found, as `(field, why)`.
pub fn parse_settings(body: &[u8]) -> Result<Settings, Vec<Problem>> {
	let settings: Settings = read(body, SETTINGS_BODY_CEILING)?;
	let mut problems = Vec::new();
	let ids: Vec<&str> = settings.checkpoints.iter().map(|checkpoint| checkpoint.id.as_str()).collect();
	if settings.checkpoints.len() > MAX_CHECKPOINTS {
		problems.push(("checkpoints", "holds more than 12 checkpoints"));
	}
	if ids.iter().any(|id| !curriculum_repo::is_plain_key(id)) {
		problems.push(("checkpoints", "each checkpoint id is a plain key"));
	}
	if ids.iter().enumerate().any(|(i, id)| ids[..i].contains(id)) {
		problems.push(("checkpoints", "each checkpoint id is used once"));
	}
	if settings.checkpoints.iter().any(|checkpoint| !(0..MINUTES_IN_A_DAY).contains(&checkpoint.minutes)) {
		problems.push(("checkpoints", "each checkpoint's minutes fall within one day"));
	}
	if settings.checkpoints.iter().any(|checkpoint| too_long(&checkpoint.label, MAX_LABEL_CHARS)) {
		problems.push(("checkpoints", "each checkpoint's label is at most 64 characters"));
	}
	if let Some(why) = labels_problem(&settings.labels) {
		problems.push(("labels", why));
	}
	if !is_day(&settings.since) {
		problems.push(("since", NOT_A_DAY));
	}
	if problems.is_empty() {
		Ok(settings)
	} else {
		Err(problems)
	}
}

#[cfg(test)]
mod tests {
	use super::{breaches, parse_entry, parse_settings, Checkpoint, Entry, Mine, Settings, Theirs};
	use serde_json::json;

	pub(crate) fn settings() -> Settings {
		Settings {
			checkpoints: vec![
				Checkpoint {
					id: "7".to_owned(),
					label: "7:00".to_owned(),
					minutes: 420,
					goal: 4100,
				},
				Checkpoint {
					id: "12".to_owned(),
					label: "12:00".to_owned(),
					minutes: 720,
					goal: 5100,
				},
			],
			labels: vec!["no-bs".to_owned()],
			step: 100,
			usual_low: 3000,
			usual_high: 6500,
			tolerance: 100,
			since: "2026-09-14".to_owned(),
		}
	}

	fn entry() -> Entry {
		Entry {
			id: "e1".to_owned(),
			day: "2026-10-17".to_owned(),
			checkpoint: Some("7".to_owned()),
			time: None,
			mine: Some(Mine { value: 4300, approx: true }),
			theirs: None,
			goal: Some(4100),
			labels: vec![],
			note: None,
			review: None,
		}
	}

	fn fields(problems: &[(&'static str, &'static str)]) -> Vec<&'static str> {
		let mut named: Vec<&str> = problems.iter().map(|(field, _)| *field).collect();
		named.sort_unstable();
		named.dedup();
		named
	}

	#[test]
	fn the_phones_own_entry_shape_reads_and_round_trips() {
		let body = json!({
			"id": "e1", "day": "2026-10-17", "checkpoint": "7", "time": null,
			"mine": { "value": 4300, "approx": true }, "theirs": null, "goal": 4100,
			"labels": [], "note": null, "review": null
		})
		.to_string();
		let read = parse_entry("e1", body.as_bytes(), Some(&settings())).unwrap();
		assert_eq!(read, entry());
		assert_eq!(serde_json::to_value(&read).unwrap(), serde_json::from_str::<serde_json::Value>(&body).unwrap());
	}

	#[test]
	fn an_entry_breaking_a_rule_of_its_own_names_each_field() {
		let refused = |value: serde_json::Value, settings: Option<&Settings>| fields(&parse_entry("e1", value.to_string().as_bytes(), settings).unwrap_err());
		let mut base = serde_json::to_value(entry()).unwrap();
		assert_eq!(refused(base.clone(), None), ["checkpoint"], "no settings, so no checkpoint is one of them");
		base["id"] = json!("e2");
		base["day"] = json!("17 Oct");
		base["mine"] = json!(null);
		base["goal"] = json!(null);
		base["time"] = json!("7:00");
		base["checkpoint"] = json!("9");
		assert_eq!(refused(base, Some(&settings())), ["checkpoint", "day", "goal", "id", "mine", "time"]);

		let mut off = serde_json::to_value(entry()).unwrap();
		off["checkpoint"] = json!(null);
		assert_eq!(refused(off, Some(&settings())), ["goal"], "a goal off the checkpoints");
		let mut fractional = serde_json::to_value(entry()).unwrap();
		fractional["mine"] = json!({ "value": 4300.5, "approx": false });
		assert_eq!(refused(fractional, Some(&settings())), ["body"]);
		let mut extra = serde_json::to_value(entry()).unwrap();
		extra["mood"] = json!("fine");
		assert_eq!(refused(extra, Some(&settings())), ["body"], "nothing the record does not describe");
		assert_eq!(fields(&parse_entry("e.json", b"{}", None).unwrap_err()), ["body"]);
	}

	#[test]
	fn a_revision_keeps_where_and_when_and_their_figure() {
		let was = Entry {
			theirs: Some(Theirs { value: 4250 }),
			..entry()
		};
		let mut is = was.clone();
		is.labels = vec!["no-bs".to_owned()];
		is.note = Some("late".to_owned());
		is.mine = None;
		assert!(breaches(&was, &is).is_empty(), "labels, the note and mine may change");
		is.day = "2026-10-18".to_owned();
		is.goal = Some(1);
		is.theirs = None;
		assert_eq!(fields(&breaches(&was, &is)), ["day", "goal", "theirs"]);
		let unreported = entry();
		let reported = Entry {
			theirs: Some(Theirs { value: 1 }),
			..entry()
		};
		assert!(breaches(&unreported, &reported).is_empty(), "their figure may come in");
	}

	#[test]
	fn a_day_is_one_the_calendar_has() {
		for day in ["2026-10-17", "2028-02-29", "2000-02-29", "2026-12-31"] {
			assert!(super::is_day(day), "{day}");
		}
		for day in [
			"2026-99-99",
			"2026-02-31",
			"2026-02-29",
			"1900-02-29",
			"2026-00-10",
			"2026-04-31",
			"2026-10-00",
			"2026/10/17",
			"26-10-17",
		] {
			assert!(!super::is_day(day), "{day}");
		}
	}

	#[test]
	fn settings_read_and_refuse_by_field() {
		let body = serde_json::to_value(settings()).unwrap().to_string();
		assert_eq!(parse_settings(body.as_bytes()).unwrap(), settings());
		let mut wrong = serde_json::to_value(settings()).unwrap();
		wrong["checkpoints"][1]["id"] = json!("7");
		wrong["since"] = json!("soon");
		wrong["labels"] = json!([""]);
		assert_eq!(fields(&parse_settings(wrong.to_string().as_bytes()).unwrap_err()), ["checkpoints", "labels", "since"]);
	}
}
