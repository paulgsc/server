//! What a video's DNA is in its home form, and the checks a queued DNA
//! passes before it is stored. Pure: no database.
//!
//! The wire form is `some-ui.video-dna/1`, the one paulgsc/some-ui ST33
//! (#2591) sends: one JSON object, its keys `camelCase`. Home checks the
//! fields listed below and keeps the bytes as sent; `shape` and `data` must be
//! JSON objects, and home reads nothing inside them.

use serde::Deserialize;
use serde_json::Value;
use studio_log_repo::instant;

/// `(field, why)`: one reason a write was refused.
pub type Problem = (&'static str, &'static str);

/// The one schema home takes.
pub const SCHEMA: &str = "some-ui.video-dna/1";

/// The most bytes one DNA may hold. A shape and its data are text: a cut
/// list, a pin and each scene's point and facts, well under this.
pub const DNA_CEILING: usize = 256 * 1024;

/// The most characters a title holds, after trimming.
pub(crate) const MAX_TITLE_CHARS: usize = 200;

/// The most takes one DNA names. Each is checked against the voice notes
/// home holds, in the transaction that writes, so the count is bounded on its
/// own as well as by [`DNA_CEILING`].
pub(crate) const MAX_TAKES: usize = 200;

const MAX_ID_CHARS: usize = 64;

/// Which template a video goes through, as the DNA names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Template {
	/// `show-format` today; ST29's templates widen it.
	pub(crate) kind: String,
	pub(crate) id: String,
	pub(crate) version: i64,
}

/// One DNA, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Dna {
	pub(crate) video: String,
	pub(crate) rev: i64,
	/// Trimmed.
	pub(crate) title: String,
	/// ISO-8601 UTC to the millisecond.
	pub(crate) queued_at: String,
	pub(crate) template: Template,
	/// Each voice note the takes name, once, in the order first named.
	pub(crate) notes: Vec<String>,
	/// The bytes as sent.
	pub(crate) text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireTemplate {
	kind: String,
	id: String,
	version: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireTake {
	scene: String,
	note: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Wire {
	schema: String,
	video: String,
	rev: i64,
	title: String,
	queued_at: String,
	template: WireTemplate,
	takes: Vec<WireTake>,
	shape: Value,
	data: Value,
}

/// The phone's own ids (`crypto.randomUUID()` today): URL-safe, short.
fn is_id(text: &str) -> bool {
	!text.is_empty() && text.len() <= MAX_ID_CHARS && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// A name the code gives a template, its kind or a scene: lowercase letters,
/// digits and `-`, starting with a letter or digit, at most 64 characters
/// (paulgsc/some-ui `@some-ui/video-render`'s `showId` and `sceneId`).
fn is_name(text: &str) -> bool {
	text.len() <= MAX_ID_CHARS
		&& text.bytes().next().is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
		&& text.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// A revision as the path writes it: a whole number from 1, digits only.
fn revision(text: &str) -> Option<i64> {
	if text.is_empty() || text.starts_with('0') || !text.bytes().all(|byte| byte.is_ascii_digit()) {
		return None;
	}
	text.parse().ok()
}

fn title(problems: &mut Vec<Problem>, sent: &str) -> String {
	let trimmed = sent.trim();
	if trimmed.is_empty() || trimmed.chars().count() > MAX_TITLE_CHARS || trimmed.chars().any(char::is_control) {
		problems.push(("title", "must be 1 to 200 characters once trimmed, on one line"));
	}
	trimmed.to_owned()
}

fn template(problems: &mut Vec<Problem>, sent: WireTemplate) -> Template {
	if !is_name(&sent.kind) || !is_name(&sent.id) {
		problems.push(("template", "its kind and id must each be 1 to 64 lowercase letters, digits or `-`"));
	}
	if sent.version < 1 {
		problems.push(("template", "its version must be a whole number from 1"));
	}
	Template {
		kind: sent.kind,
		id: sent.id,
		version: sent.version,
	}
}

fn takes(problems: &mut Vec<Problem>, sent: &[WireTake]) -> Vec<String> {
	if sent.len() > MAX_TAKES {
		problems.push(("takes", "holds at most 200 takes"));
	}
	if !sent.iter().all(|take| is_name(&take.scene) && voice_repo::is_note_id(&take.note)) {
		problems.push(("takes", "each names a scene and a voice note by the phone's id"));
	}
	let mut notes: Vec<String> = Vec::new();
	for take in sent {
		if !notes.contains(&take.note) {
			notes.push(take.note.clone());
		}
	}
	notes
}

/// The DNA in `body`, queued as revision `rev` of `video` (each as the path
/// writes it), checked against every rule that needs no database. Which
/// voice notes home holds is the store's to check.
///
/// # Errors
/// Every problem found, as `(field, why)`, each field once per reason.
pub(crate) fn parse_dna(video: &str, rev: &str, body: &[u8]) -> Result<Dna, Vec<Problem>> {
	if body.len() > DNA_CEILING {
		return Err(vec![("body", "is over 256 KiB")]);
	}
	// Straight from the bytes, so a key written twice is refused rather than
	// read as its last copy.
	let off_shape = || vec![("body", "is not the shape a video's DNA holds")];
	let wire: Wire = serde_json::from_slice(body).map_err(|_| off_shape())?;
	// JSON that parses is UTF-8; kept as text, byte for byte.
	let text = std::str::from_utf8(body).map_err(|_| off_shape())?.to_owned();
	let mut problems = Vec::new();
	if wire.schema != SCHEMA {
		problems.push(("schema", "must be `some-ui.video-dna/1`"));
	}
	if !is_id(video) {
		problems.push(("video", "must be the phone's id: 1 to 64 letters, digits, `-` or `_`"));
	} else if wire.video != video {
		problems.push(("video", "must be the video the path names"));
	}
	match revision(rev) {
		None => problems.push(("rev", "must be a whole number from 1")),
		Some(path_rev) if path_rev != wire.rev => problems.push(("rev", "must be the revision the path names")),
		Some(_) => {}
	}
	let title = title(&mut problems, &wire.title);
	let queued_at = instant(&wire.queued_at).unwrap_or_else(|| {
		problems.push(("queuedAt", "must be an RFC 3339 instant in UTC, ending `Z`"));
		String::new()
	});
	let template = template(&mut problems, wire.template);
	let notes = takes(&mut problems, &wire.takes);
	if !wire.shape.is_object() {
		problems.push(("shape", "must be a JSON object"));
	}
	if !wire.data.is_object() {
		problems.push(("data", "must be a JSON object"));
	}
	problems.dedup();
	if !problems.is_empty() {
		return Err(problems);
	}
	Ok(Dna {
		video: wire.video,
		rev: wire.rev,
		title,
		queued_at,
		template,
		notes,
		text,
	})
}

#[cfg(test)]
mod tests {
	use super::{parse_dna, DNA_CEILING, MAX_TAKES, MAX_TITLE_CHARS};
	use serde_json::{json, Value};

	fn dna() -> Value {
		json!({
			"schema": "some-ui.video-dna/1",
			"video": "v1",
			"rev": 3,
			"title": "Playoff Delta · 2026 week 8",
			"queuedAt": "2026-11-05T17:02:11.123Z",
			"template": { "kind": "show-format", "id": "playoff-delta", "version": 2 },
			"takes": [{ "scene": "division", "note": "n1" }, { "scene": "outro", "note": "n2" }, { "scene": "recap", "note": "n1" }],
			"shape": { "scenes": [{ "id": "division" }] },
			"data": { "pin": { "period": "2026-w8" } }
		})
	}

	fn fields(value: &Value) -> Vec<&'static str> {
		let mut fields: Vec<&'static str> = parse_dna("v1", "3", value.to_string().as_bytes()).unwrap_err().iter().map(|(field, _)| *field).collect();
		fields.sort_unstable();
		fields.dedup();
		fields
	}

	#[test]
	fn a_dna_reads_its_checked_fields_and_keeps_its_bytes() {
		let sent = dna().to_string();
		let read = parse_dna("v1", "3", sent.as_bytes()).unwrap();
		assert_eq!((read.video.as_str(), read.rev), ("v1", 3));
		assert_eq!(read.title, "Playoff Delta · 2026 week 8");
		assert_eq!(read.queued_at, "2026-11-05T17:02:11.123Z");
		assert_eq!(
			(read.template.kind.as_str(), read.template.id.as_str(), read.template.version),
			("show-format", "playoff-delta", 2)
		);
		assert_eq!(read.notes, ["n1", "n2"], "each note once, in the order first named");
		assert_eq!(read.text, sent, "byte for byte");

		let mut bare = dna();
		bare["takes"] = json!([]);
		bare["shape"] = json!({});
		bare["data"] = json!({});
		assert!(
			parse_dna("v1", "3", bare.to_string().as_bytes()).unwrap().notes.is_empty(),
			"no takes, and empty objects, are a DNA"
		);
		let mut padded = dna();
		padded["title"] = json!("  A title  ");
		padded["queuedAt"] = json!("2026-11-05T17:02:11Z");
		let read = parse_dna("v1", "3", padded.to_string().as_bytes()).unwrap();
		assert_eq!(
			(read.title.as_str(), read.queued_at.as_str()),
			("A title", "2026-11-05T17:02:11.000Z"),
			"trimmed, and to the millisecond"
		);
	}

	#[test]
	fn the_title_is_one_to_two_hundred_characters_once_trimmed() {
		let mut long = dna();
		long["title"] = json!("한".repeat(MAX_TITLE_CHARS));
		assert!(parse_dna("v1", "3", long.to_string().as_bytes()).is_ok(), "characters, not bytes");
		long["title"] = json!("x".repeat(MAX_TITLE_CHARS + 1));
		assert_eq!(fields(&long), ["title"]);
		long["title"] = json!("   ");
		assert_eq!(fields(&long), ["title"], "blank");
		long["title"] = json!("one\ntwo");
		assert_eq!(fields(&long), ["title"], "one line");
	}

	#[test]
	fn the_path_names_the_video_and_revision() {
		assert_eq!(
			parse_dna("v2", "3", dna().to_string().as_bytes()).unwrap_err(),
			[("video", "must be the video the path names")]
		);
		assert_eq!(
			parse_dna("v1", "4", dna().to_string().as_bytes()).unwrap_err(),
			[("rev", "must be the revision the path names")]
		);
		for rev in ["0", "03", "-3", "three", "", "99999999999999999999"] {
			assert_eq!(
				parse_dna("v1", rev, dna().to_string().as_bytes()).unwrap_err(),
				[("rev", "must be a whole number from 1")],
				"{rev:?}"
			);
		}
		assert_eq!(fields(&json!(null)), ["body"]);
		assert_eq!(
			parse_dna("v/1", "3", dna().to_string().as_bytes()).unwrap_err(),
			[("video", "must be the phone's id: 1 to 64 letters, digits, `-` or `_`")]
		);
	}

	#[test]
	fn each_checked_field_is_held_to_its_rule() {
		let mut bad = dna();
		bad["schema"] = json!("some-ui.video-dna/2");
		bad["queuedAt"] = json!("2026-11-05T09:02:11-08:00");
		bad["template"] = json!({ "kind": "Show Format", "id": "playoff-delta", "version": 0 });
		bad["takes"] = json!([{ "scene": "division", "note": "n/1" }]);
		bad["shape"] = json!([]);
		bad["data"] = json!("filled later");
		assert_eq!(fields(&bad), ["data", "queuedAt", "schema", "shape", "takes", "template"]);
		let mut scene = dna();
		scene["takes"] = json!([{ "scene": "-division", "note": "n1" }]);
		assert_eq!(fields(&scene), ["takes"], "a scene's id is the code's name");
		let mut many = dna();
		many["takes"] = (0..=MAX_TAKES).map(|n| json!({ "scene": "s", "note": String::from("n") + &n.to_string() })).collect();
		assert_eq!(fields(&many), ["takes"]);
	}

	#[test]
	fn the_shape_of_the_whole_is_held() {
		let mut extra = dna();
		extra["filled"] = json!(true);
		assert_eq!(fields(&extra), ["body"], "no field version 1 does not carry");
		let mut missing = dna();
		missing.as_object_mut().unwrap().remove("data");
		assert_eq!(fields(&missing), ["body"], "every field version 1 carries");
		let mut take = dna();
		take["takes"][0]["words"] = json!("what I said");
		assert_eq!(fields(&take), ["body"], "a take is named by its note only");
		let mut template = dna();
		template["template"]["assets"] = json!([]);
		assert_eq!(fields(&template), ["body"]);
		let twice = dna().to_string().replacen("\"rev\":3", "\"rev\":3,\"rev\":3", 1);
		assert!(parse_dna("v1", "3", twice.as_bytes()).is_err(), "a key written twice");
		let big = json!({ "padding": "x".repeat(DNA_CEILING) });
		assert_eq!(parse_dna("v1", "3", big.to_string().as_bytes()).unwrap_err(), [("body", "is over 256 KiB")]);
	}
}
