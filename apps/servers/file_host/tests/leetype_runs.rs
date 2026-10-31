//! What still holds the `LeetType` runs to `docs/leetype-execution.md`'s nevers
//! once no route serves them (paulgsc/some-ui#2032 retired
//! `GET /leetype/rounds*`). The recordings stay in `leetype_round_run`, written
//! by `record-leetype-runs` and read by `dump-leetype-snapshot`, so never #3
//! (no complexity claim in a `RoundRuns`), never #5 (no `StudySignal`) and EX1
//! (nothing the server serves compiles or runs a program) still have a
//! surface to hold. These tests moved here from the retired route's handler.

use leetype_round_repo::{Bounds, Elapsed, ErrorClass, ExecutionError, Observation, RecordedRun, RoundRuns, RunResult, Variant};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// A transcript with one run of each kind: `A` finished at `C`, and the
/// three ways a run fails.
fn transcript() -> Vec<RecordedRun> {
	let error = |variant, bounds, error_class| RecordedRun {
		variant,
		bounds,
		sizes: BTreeMap::from([(String::from("n"), 100_000), (String::from("q"), 100_000)]),
		result: RunResult::Error {
			input_size: 100_000,
			error: ExecutionError {
				error_class,
				message: String::from("detail"),
			},
		},
	};
	vec![
		RecordedRun {
			variant: Variant::Algorithm,
			bounds: Bounds::Before,
			sizes: BTreeMap::from([(String::from("n"), 1_000), (String::from("q"), 1_000)]),
			result: RunResult::Ok {
				input_size: 1_000,
				observation: Observation {
					output: String::from("false"),
					logs: vec![String::from("a note")],
					elapsed: Elapsed { milliseconds: 4 },
				},
			},
		},
		error(Variant::Algorithm, Bounds::After, ErrorClass::BudgetExceeded),
		error(Variant::Diff(0), Bounds::Before, ErrorClass::Compile),
		error(Variant::Diff(1), Bounds::After, ErrorClass::Runtime),
	]
}

/// Never #3: every key a `RoundRuns` can carry (what `dump-leetype-snapshot`
/// writes as `runs/<id>.json`), walked through a sample with each branch of
/// `RunResult`, is on this list — none of them a class, a Θ, an
/// "admissible", or any other claim. A new field fails here and has to be
/// argued for. (`sizes`' keys are dimension names from the round, not
/// fields, and are not walked.)
#[test]
fn a_runs_transcript_carries_no_complexity_claim() {
	const ALLOWED: [&str; 17] = [
		"roundId",
		"contentHash",
		"runs",
		"variant",
		"bounds",
		"sizes",
		"result",
		"kind",
		"inputSize",
		"observation",
		"output",
		"logs",
		"elapsed",
		"milliseconds",
		"error",
		"errorClass",
		"message",
	];
	fn keys(value: &Value, found: &mut BTreeSet<String>) {
		match value {
			Value::Object(map) => {
				for (key, value) in map {
					found.insert(key.clone());
					if key != "sizes" {
						keys(value, found);
					}
				}
			}
			Value::Array(items) => items.iter().for_each(|item| keys(item, found)),
			Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
		}
	}
	let sample = RoundRuns {
		round_id: String::from("a"),
		content_hash: String::from("h"),
		runs: transcript(),
	};
	let mut found = BTreeSet::new();
	keys(&serde_json::to_value(&sample).unwrap(), &mut found);
	let allowed: BTreeSet<String> = ALLOWED.iter().map(|key| (*key).to_owned()).collect();
	assert_eq!(found, allowed, "every allowed key is exercised, and nothing else appears");
	let kinds: BTreeSet<&str> = sample
		.runs
		.iter()
		.map(|run| if matches!(run.result, RunResult::Ok { .. }) { "ok" } else { "error" })
		.collect();
	assert_eq!(kinds.len(), 2, "both branches walked");
}

/// The code lines of `source`, comments dropped, and of a file with tests
/// only what precedes them.
fn code(source: &str) -> String {
	let source = source.split(concat!("#[cfg(", "test)]")).next().unwrap_or_default();
	source.lines().filter(|line| !line.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n")
}

/// Never #5, and EX1 ("no execution in the request path"), as checks over
/// the source:
///
/// - the runs surface — both commands — names nothing that produces or
///   carries a `StudySignal` (`study_domain`, `handlers::signals`, the nudge,
///   outcomes, sessions, engagement). `leetype_runner`'s own closure is
///   checked by its `tests/nevers.rs`;
/// - no file of the server but `record-leetype-runs` names the runner, so
///   nothing the server serves can compile or execute a program.
#[test]
fn the_runs_surface_reaches_neither_a_study_signal_nor_the_runner() {
	let surface = [
		("bin/record_leetype_runs.rs", include_str!("../src/bin/record_leetype_runs.rs")),
		("bin/dump_leetype_snapshot.rs", include_str!("../src/bin/dump_leetype_snapshot.rs")),
	];
	let forbidden = [
		["Study", "Signal"].concat(),
		["study", "_domain"].concat(),
		["handlers::", "signals"].concat(),
		["crate::", "nudge"].concat(),
		["file_host::", "nudge"].concat(),
		["outcome", "_repo"].concat(),
		["session", "_repo"].concat(),
		["engagement", "_repo"].concat(),
		["intervention", "::"].concat(),
	];
	for (file, source) in surface {
		let code = code(source);
		for word in &forbidden {
			assert!(!code.contains(word.as_str()), "{file} names {word}");
		}
	}

	let runner = ["leetype", "_runner"].concat();
	let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
	let mut stack = vec![root.clone()];
	let mut scanned = 0;
	while let Some(dir) = stack.pop() {
		for entry in std::fs::read_dir(&dir).unwrap() {
			let path = entry.unwrap().path();
			if path.is_dir() {
				stack.push(path);
			} else if path.extension().is_some_and(|extension| extension == "rs") {
				scanned += 1;
				let relative = path.strip_prefix(&root).unwrap().to_string_lossy().into_owned();
				let names_runner = code(&std::fs::read_to_string(&path).unwrap()).contains(runner.as_str());
				assert_eq!(names_runner, relative == "bin/record_leetype_runs.rs", "{relative}");
			}
		}
	}
	assert!(scanned > 50, "the tree was walked: {scanned} files");
}
