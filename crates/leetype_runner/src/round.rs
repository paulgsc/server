//! The part of a round body the runner reads, and the programs it builds
//! from it.
//!
//! A round body is `@some-ui/leetype`'s `RoundSchema`, stored verbatim. The
//! runner reads `algorithm.source` (`A`), both constraint sets, each option's
//! hunk, and `harness.source`; nothing else, and it never writes a body.

use leetype_round_repo::{Bounds, Variant};
use serde::Deserialize;

/// A round, as far as running it goes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnableRound {
	pub id: String,
	pub algorithm: Algorithm,
	pub constraint_diff: ConstraintDiff,
	pub diff_options: Vec<DiffOption>,
	/// Absent on a round authored before harnesses existed: such a round
	/// cannot be run, and is skipped rather than failed.
	pub harness: Option<Harness>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Algorithm {
	/// `A`: a complete Rust item (the entry point and what it needs), no
	/// `main`.
	pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ConstraintDiff {
	/// `C`, under which `A` is admissible.
	pub before: Vec<Constraint>,
	/// `C′`, under which it is not.
	pub after: Vec<Constraint>,
}

impl ConstraintDiff {
	/// The constraint set `bounds` names.
	#[must_use]
	pub fn set(&self, bounds: Bounds) -> &[Constraint] {
		match bounds {
			Bounds::Before => &self.before,
			Bounds::After => &self.after,
		}
	}
}

/// One symbolic input bound, `dimension <= bound`.
///
/// The harness is given the bound itself: admissibility is a worst-case relation (Def. 3.1), so the
/// largest input the constraint allows is the one that says something.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Constraint {
	pub dimension: String,
	pub bound: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DiffOption {
	pub member: Member,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Member {
	pub hunk: Hunk,
}

/// A Def. 1.4 hunk against `A`, as `types/exercise.ts`'s `DiffHunk` spells
/// it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hunk {
	pub old_start: i64,
	pub new_start: i64,
	pub segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Segment {
	pub kind: SegmentKind,
	pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SegmentKind {
	Context,
	Deletion,
	Addition,
}

/// The round's worst-case driver.
///
/// Rust appended after `A` (or `A + d`) to make a binary whose `main` reads one `<dimension>=<n>` argument per
/// constrained dimension, builds a worst-case input, calls the entry point,
/// and prints one short line.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Harness {
	pub source: String,
}

/// Apply `hunk` to `source` exactly as the client's `applyHunk`
/// (`lib/leetype/round-assembly`) does.
///
/// The hunk's context and deletion segments, concatenated, must appear verbatim starting at line `oldStart`
/// (numbered from 1), and are replaced by its context and addition segments;
/// `newStart` must equal `oldStart`.
///
/// # Errors
/// Why the hunk does not apply, in the client's words.
pub fn apply_hunk(source: &str, hunk: &Hunk) -> Result<String, String> {
	if hunk.old_start < 1 {
		let mut reason = String::from("oldStart is ");
		reason.push_str(&hunk.old_start.to_string());
		reason.push_str("; lines are numbered from 1");
		return Err(reason);
	}
	if hunk.new_start != hunk.old_start {
		let mut reason = String::from("newStart (");
		reason.push_str(&hunk.new_start.to_string());
		reason.push_str(") differs from oldStart (");
		reason.push_str(&hunk.old_start.to_string());
		reason.push_str("); a single hunk against A cannot shift its own start line");
		return Err(reason);
	}
	let old_text: String = hunk
		.segments
		.iter()
		.filter(|segment| segment.kind != SegmentKind::Addition)
		.map(|segment| segment.text.as_str())
		.collect();
	let new_text: String = hunk
		.segments
		.iter()
		.filter(|segment| segment.kind != SegmentKind::Deletion)
		.map(|segment| segment.text.as_str())
		.collect();
	if old_text == new_text {
		return Err(String::from("the hunk has no addition or deletion, so it changes nothing"));
	}
	let Some(offset) = offset_of_line(source, hunk.old_start) else {
		let mut reason = String::from("oldStart ");
		reason.push_str(&hunk.old_start.to_string());
		reason.push_str(" is past the end of the source");
		return Err(reason);
	};
	if !source[offset..].starts_with(&old_text) {
		let mut reason = String::from("the hunk's context and deletion text does not match the source at line ");
		reason.push_str(&hunk.old_start.to_string());
		return Err(reason);
	}
	let mut patched = String::with_capacity(source.len() - old_text.len() + new_text.len());
	patched.push_str(&source[..offset]);
	patched.push_str(&new_text);
	patched.push_str(&source[offset + old_text.len()..]);
	Ok(patched)
}

/// Byte offset of the start of 1-based `line` in `source`, or `None` past
/// the end. Always a character boundary: it is 0 or just after a `\n`.
fn offset_of_line(source: &str, line: i64) -> Option<usize> {
	let mut offset = 0;
	let mut current = 1;
	while current < line {
		offset += source[offset..].find('\n')? + 1;
		current += 1;
	}
	Some(offset)
}

/// One program the runner builds: its [`Variant`] and its full source, the
/// variant's code followed by a newline and the harness — the same text the
/// client's `check-round-programs-compile.ts` compiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
	pub variant: Variant,
	pub source: String,
}

/// Why a round could not be turned into programs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgramError {
	/// A hunk did not apply to `A`.
	Hunk { variant: Variant, reason: String },
	/// More options than a variant label can name. `parse_round` refuses
	/// more than five, so only a body that never went through it gets here.
	TooManyOptions(usize),
}

impl std::fmt::Display for ProgramError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Hunk { variant, reason } => write!(f, "variant {} does not apply: {reason}", variant.label()),
			Self::TooManyOptions(count) => write!(f, "{count} diff options; at most ten can be labelled"),
		}
	}
}

impl std::error::Error for ProgramError {}

/// `A` and every `A + d`, in option order, each with `harness` appended.
/// Every hunk is applied before anything is compiled, so a round whose hunks
/// do not all apply is refused whole.
///
/// # Errors
/// The first hunk that does not apply, naming its variant.
pub fn programs(round: &RunnableRound, harness: &Harness) -> Result<Vec<Program>, ProgramError> {
	let with_harness = |code: &str| {
		let mut source = String::with_capacity(code.len() + 1 + harness.source.len());
		source.push_str(code);
		source.push('\n');
		source.push_str(&harness.source);
		source
	};
	let mut programs = vec![Program {
		variant: Variant::Algorithm,
		source: with_harness(&round.algorithm.source),
	}];
	for (index, option) in round.diff_options.iter().enumerate() {
		let variant = u8::try_from(index)
			.ok()
			.filter(|index| *index < 10)
			.map(Variant::Diff)
			.ok_or(ProgramError::TooManyOptions(round.diff_options.len()))?;
		let patched = apply_hunk(&round.algorithm.source, &option.member.hunk).map_err(|reason| ProgramError::Hunk { variant, reason })?;
		programs.push(Program {
			variant,
			source: with_harness(&patched),
		});
	}
	Ok(programs)
}

#[cfg(test)]
mod tests {
	use super::{apply_hunk, programs, Harness, Hunk, ProgramError, RunnableRound, Segment, SegmentKind};
	use leetype_round_repo::Variant;
	use serde_json::json;

	const A: &str = "fn f(v: &[i64]) -> i64 {\n    let mut t = 0;\n    for x in v {\n        t += x;\n    }\n    t\n}\n";

	fn segment(kind: SegmentKind, text: &str) -> Segment {
		Segment { kind, text: text.to_owned() }
	}

	fn hunk(old_start: i64, new_start: i64, segments: Vec<Segment>) -> Hunk {
		Hunk { old_start, new_start, segments }
	}

	#[test]
	fn a_hunk_replaces_its_context_and_deletion_at_its_line() {
		let applied = apply_hunk(
			A,
			&hunk(
				3,
				3,
				vec![
					segment(SegmentKind::Context, "    for x in v {\n"),
					segment(SegmentKind::Deletion, "        t += x;\n"),
					segment(SegmentKind::Addition, "        t += 2 * x;\n"),
					segment(SegmentKind::Context, "    }\n"),
				],
			),
		)
		.unwrap();
		assert_eq!(applied, A.replace("t += x;", "t += 2 * x;"));
		let first_line = apply_hunk(A, &hunk(1, 1, vec![segment(SegmentKind::Deletion, "fn f"), segment(SegmentKind::Addition, "pub fn f")])).unwrap();
		assert!(first_line.starts_with("pub fn f(v"));
	}

	#[test]
	fn a_hunk_that_does_not_apply_says_why_as_the_client_does() {
		let cases = [
			(hunk(0, 0, vec![segment(SegmentKind::Addition, "x")]), "oldStart is 0; lines are numbered from 1"),
			(
				hunk(2, 3, vec![segment(SegmentKind::Addition, "x")]),
				"newStart (3) differs from oldStart (2); a single hunk against A cannot shift its own start line",
			),
			(
				hunk(2, 2, vec![segment(SegmentKind::Context, "    let mut t = 0;\n")]),
				"the hunk has no addition or deletion, so it changes nothing",
			),
			(hunk(40, 40, vec![segment(SegmentKind::Addition, "x")]), "oldStart 40 is past the end of the source"),
			(
				hunk(2, 2, vec![segment(SegmentKind::Deletion, "    for x in v {\n")]),
				"the hunk's context and deletion text does not match the source at line 2",
			),
		];
		for (hunk, reason) in cases {
			assert_eq!(apply_hunk(A, &hunk).unwrap_err(), reason);
		}
	}

	fn round(hunks: &serde_json::Value) -> RunnableRound {
		let options: Vec<serde_json::Value> = hunks
			.as_array()
			.unwrap()
			.iter()
			.map(|hunk| json!({ "member": { "hunk": hunk, "propositionId": "CW-P1", "admissible": false } }))
			.collect();
		serde_json::from_value(json!({
			"id": "r", "algorithm": { "language": "rust", "source": A },
			"constraintDiff": { "before": [{ "dimension": "n", "operator": "<=", "bound": 10 }], "after": [] },
			"diffOptions": options, "harness": { "source": "fn main() {}\n" }
		}))
		.unwrap()
	}

	/// `A` first, then each `A + d` in option order, each followed by a
	/// newline and the harness; a hunk that does not apply refuses the round
	/// and names its variant.
	#[test]
	fn programs_are_a_then_each_patched_variant_with_the_harness() {
		let good = json!({ "oldStart": 6, "newStart": 6, "segments": [
			{ "kind": "deletion", "text": "    t\n" }, { "kind": "addition", "text": "    t + 1\n" }] });
		let harness = Harness {
			source: String::from("fn main() {}\n"),
		};
		let built = programs(&round(&json!([good, good])), &harness).unwrap();
		let variants: Vec<Variant> = built.iter().map(|program| program.variant).collect();
		assert_eq!(variants, [Variant::Algorithm, Variant::Diff(0), Variant::Diff(1)]);
		assert_eq!(built[0].source, String::from(A) + "\nfn main() {}\n");
		assert!(built[1].source.contains("    t + 1\n}\n\nfn main() {}\n"));

		let bad = json!({ "oldStart": 5, "newStart": 5, "segments": [{ "kind": "deletion", "text": "nope\n" }] });
		let err = programs(&round(&json!([good, bad])), &harness).unwrap_err();
		assert_eq!(
			err,
			ProgramError::Hunk {
				variant: Variant::Diff(1),
				reason: String::from("the hunk's context and deletion text does not match the source at line 5")
			}
		);
		assert!(err.to_string().starts_with("variant d1 does not apply"), "{err}");
	}
}
