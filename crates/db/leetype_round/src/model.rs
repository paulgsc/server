use serde::Serialize;
use serde_json::{Map, Value};

pub use curriculum_repo::{content_hash, is_plain_key};

/// The largest round body this server stores, in bytes.
///
/// A round is one program, two constraint sets, a cost graph, and a handful of
/// hunks with their own graphs: a few kilobytes (the authored ones are about
/// six). The ceiling is on the one input stored without being understood, so
/// a mistaken paste of something else is refused rather than kept forever.
pub const ROUND_BYTES_CEILING: usize = 256 * 1024;

/// The fewest members a round's option set `D` may have: one admissible
/// member and at least one distractor, or there is nothing to select between.
pub const MIN_DIFF_OPTIONS: usize = 2;

/// One member of a round's option set, as the edge table holds it: `μ` of
/// that member, and whether it is the admissible one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Witness {
	/// `CW-P<n>`, the canon register entry the member witnesses.
	pub proposition_id: String,
	pub admissible: bool,
}

/// What the server reads out of a round body — the only fields it reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRound {
	/// `Round.id`.
	pub id: String,
	/// `algorithm.language`; always `rust` once parsed.
	pub language: String,
	/// `diffOptions[i].member`'s `μ` and admissibility, in option order.
	pub witnesses: Vec<Witness>,
}

/// One round as this server tracks it: everything but the body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundEntry {
	pub id: String,
	pub version: i64,
	pub published_at: String,
	pub content_hash: String,
	/// When the operator retired it (ISO-8601 UTC), or `None` while it is
	/// listed.
	pub retired_at: Option<String>,
	/// In option order.
	pub witnesses: Vec<Witness>,
}

/// A field of a round body this server refuses, and why.
///
/// The field is a path rooted at `body` (`body.diffOptions[1].member.admissible`):
/// the body is what the operator's write route calls it, and what the importer
/// reads from one file.
pub type Problem = (String, &'static str);

/// Whether `id` is a register identifier: `CW-P` and a number from 1 to 999
/// with no leading zero.
///
/// The form only. Whether the canon's register defines that number is the
/// client's corpus lint to check — the register is the canon's, and a copy of
/// it here would drift from it.
#[must_use]
pub fn is_proposition_id(id: &str) -> bool {
	id.strip_prefix("CW-P")
		.is_some_and(|number| (1..=3).contains(&number.len()) && number.bytes().all(|byte| byte.is_ascii_digit()) && !number.starts_with('0'))
}

/// Read a round body: the **only** validation this server does on one.
///
/// A JSON object no larger than [`ROUND_BYTES_CEILING`], with a string `id`,
/// `algorithm.language` equal to `rust`, and a `diffOptions` array of at least
/// [`MIN_DIFF_OPTIONS`] members whose `member.propositionId` is a register
/// identifier ([`is_proposition_id`]) and `member.admissible` a boolean, true
/// for exactly one of them. Nothing else is read: programs, constraint sets,
/// budgets, cost graphs and hunks are `@some-ui/leetype`'s, and a new field
/// there changes nothing here.
///
/// # Errors
/// Every problem found, each naming its field, so one refusal says all of
/// what is wrong.
pub fn parse_round(body: &[u8]) -> Result<ParsedRound, Vec<Problem>> {
	if body.len() > ROUND_BYTES_CEILING {
		return Err(vec![(String::from("body"), "is over the round size ceiling")]);
	}
	let Ok(Value::Object(round)) = serde_json::from_slice::<Value>(body) else {
		return Err(vec![(String::from("body"), "is not a JSON object")]);
	};

	let mut problems: Vec<Problem> = Vec::new();
	let id = round.get("id").and_then(Value::as_str);
	if id.is_none() {
		problems.push((String::from("body.id"), "must be a string"));
	}
	let language = round.get("algorithm").and_then(|algorithm| algorithm.get("language")).and_then(Value::as_str);
	if language != Some("rust") {
		problems.push((String::from("body.algorithm.language"), "must be \"rust\""));
	}

	let mut witnesses = Vec::new();
	match round.get("diffOptions").and_then(Value::as_array) {
		None => problems.push((String::from("body.diffOptions"), "must be an array")),
		Some(options) if options.len() < MIN_DIFF_OPTIONS => problems.push((String::from("body.diffOptions"), "must have at least two members")),
		Some(options) => {
			for (index, option) in options.iter().enumerate() {
				if let Some(witness) = witness(option.get("member").and_then(Value::as_object), index, &mut problems) {
					witnesses.push(witness);
				}
			}
			if witnesses.len() == options.len() && witnesses.iter().filter(|witness| witness.admissible).count() != 1 {
				problems.push((String::from("body.diffOptions"), "must have exactly one admissible member"));
			}
		}
	}

	match (id, language) {
		(Some(id), Some(language)) if problems.is_empty() => Ok(ParsedRound {
			id: id.to_owned(),
			language: language.to_owned(),
			witnesses,
		}),
		_ => Err(problems),
	}
}

/// One option's `μ` and admissibility, or the problems with them.
fn witness(member: Option<&Map<String, Value>>, index: usize, problems: &mut Vec<Problem>) -> Option<Witness> {
	let field = |name: &str| {
		let mut path = String::from("body.diffOptions[");
		path.push_str(&index.to_string());
		path.push_str("].member");
		if !name.is_empty() {
			path.push('.');
			path.push_str(name);
		}
		path
	};
	let Some(member) = member else {
		problems.push((field(""), "must be an object"));
		return None;
	};
	let proposition_id = member.get("propositionId").and_then(Value::as_str).filter(|id| is_proposition_id(id));
	let admissible = member.get("admissible").and_then(Value::as_bool);
	if proposition_id.is_none() {
		problems.push((field("propositionId"), "must be a register identifier, CW-P1 to CW-P999"));
	}
	if admissible.is_none() {
		problems.push((field("admissible"), "must be a boolean"));
	}
	Some(Witness {
		proposition_id: proposition_id?.to_owned(),
		admissible: admissible?,
	})
}

/// [`parse_round`] for the round stored under `key`.
///
/// Also refuses a key that is not a plain URL path segment ([`is_plain_key`])
/// or that the body's own `id` disagrees with. The one check the importer and
/// the operator's write route share, and the one
/// [`crate::RoundRepository::upsert`] runs itself.
///
/// # Errors
/// As [`parse_round`], plus `key` and `body.id`.
pub fn validate_round(key: &str, body: &[u8]) -> Result<ParsedRound, Vec<Problem>> {
	let mut problems: Vec<Problem> = Vec::new();
	if !is_plain_key(key) {
		problems.push((String::from("key"), "must be letters, digits, - . _ or ~, not end in .json, and not be a path or URL"));
	}
	match parse_round(body) {
		Ok(round) if round.id != key => problems.push((String::from("body.id"), "must equal the round's key")),
		Ok(round) if problems.is_empty() => return Ok(round),
		Ok(_) => {}
		Err(found) => problems.extend(found),
	}
	Err(problems)
}

#[cfg(test)]
mod tests {
	use super::{is_proposition_id, parse_round, validate_round, Witness, ROUND_BYTES_CEILING};
	use serde_json::json;

	fn round(options: &serde_json::Value) -> Vec<u8> {
		json!({
			"id": "r", "algorithm": { "language": "rust", "source": "fn main() {}" },
			"constraintDiff": { "before": [], "after": [] }, "graph": { "kind": "W" },
			"diffOptions": options
		})
		.to_string()
		.into_bytes()
	}

	fn option(proposition: &str, admissible: bool) -> serde_json::Value {
		json!({ "member": { "hunk": {}, "propositionId": proposition, "admissible": admissible }, "graph": {} })
	}

	fn fields<'a>(problems: &'a [(String, &'static str)]) -> Vec<&'a str> {
		let mut fields: Vec<&str> = problems.iter().map(|(field, _)| field.as_str()).collect();
		fields.sort_unstable();
		fields
	}

	#[test]
	fn a_proposition_id_is_the_registers_form() {
		for id in ["CW-P1", "CW-P16", "CW-P999"] {
			assert!(is_proposition_id(id), "{id}");
		}
		for id in ["", "CW-P", "CW-P0", "CW-P01", "CW-P1000", "cw-p1", "CW-P1a", "P1", " CW-P1"] {
			assert!(!is_proposition_id(id), "{id}");
		}
	}

	/// Exactly `id`, `algorithm.language` and each member's `μ` and
	/// admissibility come out; nothing else is read.
	#[test]
	fn a_round_yields_its_id_and_its_members_witnesses_in_order() {
		let parsed = parse_round(&round(&json!([option("CW-P6", true), option("CW-P8", false)]))).unwrap();
		assert_eq!((parsed.id.as_str(), parsed.language.as_str()), ("r", "rust"));
		assert_eq!(
			parsed.witnesses,
			[
				Witness {
					proposition_id: "CW-P6".to_owned(),
					admissible: true
				},
				Witness {
					proposition_id: "CW-P8".to_owned(),
					admissible: false
				}
			]
		);
	}

	#[test]
	fn every_problem_is_named_by_its_field() {
		let cases: [(Vec<u8>, &[&str]); 9] = [
			(b"not json".to_vec(), &["body"]),
			(b"[1, 2]".to_vec(), &["body"]),
			(
				br#"{"algorithm":{"language":"python"},"diffOptions":[]}"#.to_vec(),
				&["body.algorithm.language", "body.diffOptions", "body.id"],
			),
			(round(&json!("nope")), &["body.diffOptions"]),
			(round(&json!([option("CW-P6", true)])), &["body.diffOptions"]),
			(round(&json!([option("CW-P6", true), option("CW-P7", true)])), &["body.diffOptions"]),
			(round(&json!([option("CW-P6", false), option("CW-P7", false)])), &["body.diffOptions"]),
			(
				round(&json!([option("CW-P0", true), { "member": { "propositionId": "CW-P2", "admissible": "yes" } }])),
				&["body.diffOptions[0].member.propositionId", "body.diffOptions[1].member.admissible"],
			),
			(round(&json!([option("CW-P6", true), { "graph": {} }])), &["body.diffOptions[1].member"]),
		];
		for (body, expected) in cases {
			let problems = parse_round(&body).unwrap_err();
			assert_eq!(fields(&problems), expected, "{}", String::from_utf8_lossy(&body));
		}
	}

	#[test]
	fn a_body_over_the_ceiling_is_refused_unread() {
		let mut body = round(&json!([option("CW-P6", true), option("CW-P8", false)]));
		body.resize(ROUND_BYTES_CEILING + 1, b' ');
		assert_eq!(fields(&parse_round(&body).unwrap_err()), ["body"]);
	}

	#[test]
	fn the_key_must_be_plain_and_the_bodys_own_id() {
		let body = round(&json!([option("CW-P6", true), option("CW-P8", false)]));
		assert!(validate_round("r", &body).is_ok());
		assert_eq!(fields(&validate_round("other", &body).unwrap_err()), ["body.id"]);
		assert_eq!(fields(&validate_round("a/b", &body).unwrap_err()), ["body.id", "key"]);
		assert_eq!(
			fields(&validate_round("a/b", b"{}").unwrap_err()),
			["body.algorithm.language", "body.diffOptions", "body.id", "key"]
		);
	}
}
