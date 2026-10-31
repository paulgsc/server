//! What holds the `LeetType` runs to `docs/leetype-execution.md`'s nevers. No
//! route serves them: the recordings are in `leetype_round_run`, written by
//! `record-leetype-runs` and read by `dump-leetype-snapshot`. These tests hold
//! never #3 (no complexity claim in a `RoundRuns`), never #5 (no
//! `StudySignal`) over that surface, and EX1 (nothing `file_host` serves
//! compiles or runs a program in a request, and `file_host` spawns no
//! compiler) over all of `src/` and the dependency graph.

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

/// Every file under `src/` that starts a process, and why it may. EX1
/// (`docs/leetype-execution.md`) is the claim; this list is its falsifier, so
/// adding to it amends EX1 and says so there.
const SPAWNS: [(&str, &str); 1] = [(
	"transcribe/engine.rs",
	"ffmpeg, a fixed decoder of a stored voice note's audio, on the transcriber's blocking thread; never a program, never in a request",
)];

/// The code lines of `source`, comments dropped, and of a file with tests
/// only what precedes them (tests may start processes). This file sits
/// outside `src/`, so the scan below never reads its own lists.
fn code(source: &str) -> String {
	let source = source.split(concat!("#[cfg(", "test)]")).next().unwrap_or_default();
	source.lines().filter(|line| !line.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n")
}

/// Never #5, and EX1 ("nothing `file_host` serves compiles or runs a program
/// in a request, and `file_host` spawns no compiler"), as checks over the
/// source:
///
/// - the runs surface — both commands — names nothing that produces or
///   carries a `StudySignal` (`study_domain`, `handlers::signals`, the nudge,
///   outcomes, sessions, engagement). `leetype_runner`'s own closure is
///   checked by its `tests/nevers.rs`;
/// - no file of the server but `record-leetype-runs` names `leetype_runner`,
///   and none names `rust_runner`, the crate that compiles and runs (the
///   `rust-runner` container does that, over its socket);
/// - no file but those in [`SPAWNS`] starts a process at all, and each of
///   those says why it may.
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
	let spawner = ["rust", "_runner"].concat();
	let spawn_markers = [["Command", "::new"].concat(), ["process::", "Command"].concat(), ["tokio::", "process"].concat()];
	let spawns_in_a_list = regex::Regex::new(&["process::\\{[^}]*\\b", "Command", "\\b"].concat()).unwrap();
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
				let code = code(&std::fs::read_to_string(&path).unwrap());
				assert_eq!(code.contains(runner.as_str()), relative == "bin/record_leetype_runs.rs", "{relative}");
				assert!(!code.contains(spawner.as_str()), "{relative} names the crate that compiles and runs programs");
				let spawns = spawn_markers.iter().any(|marker| code.contains(marker.as_str())) || spawns_in_a_list.is_match(&code);
				assert_eq!(
					spawns,
					SPAWNS.iter().any(|(file, _)| *file == relative),
					"{relative} starts a process: EX1 lists every file that may"
				);
			}
		}
	}
	assert!(scanned > 50, "the tree was walked: {scanned} files");

	// EX1 names every file that may start a process, so adding one to
	// `SPAWNS` without amending the claim fails here.
	let doc = include_str!("../docs/leetype-execution.md");
	for (file, _) in SPAWNS {
		assert!(doc.contains(&(String::from("`") + file + "`")), "EX1 does not name {file}");
	}
}

/// EX1's other half, from `cargo metadata`'s resolved graph (normal and build
/// edges; dev edges are tests'): `rust_runner`, the crate that compiles and
/// runs, reaches `file_host` only through `leetype_runner`. The source scan
/// above then holds `leetype_runner` to `record-leetype-runs`: Cargo's graph
/// is per package, not per binary, so that half is the scan's.
#[test]
fn the_runner_reaches_file_host_only_through_leetype_runner() {
	// Set by cargo for the test it runs: the toolchain's own `cargo`.
	#[allow(clippy::disallowed_methods)]
	let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
	let metadata = std::process::Command::new(cargo)
		.args(["metadata", "--format-version", "1", "--offline", "--manifest-path"])
		.arg(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
		.output()
		.unwrap();
	assert!(metadata.status.success(), "{}", String::from_utf8_lossy(&metadata.stderr));
	let metadata: Value = serde_json::from_slice(&metadata.stdout).unwrap();
	let names: BTreeMap<&str, &str> = metadata["packages"]
		.as_array()
		.unwrap()
		.iter()
		.map(|package| (package["id"].as_str().unwrap(), package["name"].as_str().unwrap()))
		.collect();
	let nodes: BTreeMap<&str, &Value> = metadata["resolve"]["nodes"]
		.as_array()
		.unwrap()
		.iter()
		.map(|node| (node["id"].as_str().unwrap(), node))
		.collect();
	let id = |name: &str| names.iter().find(|(_, package)| **package == name).map(|(id, _)| *id).unwrap();
	// What file_host reaches by normal and build edges, never passing
	// through `skip`.
	let reached = |skip: Option<&str>| {
		let mut closure = BTreeSet::new();
		let mut stack = vec![id("file_host")];
		while let Some(at) = stack.pop() {
			if Some(at) == skip || !closure.insert(at) {
				continue;
			}
			for dep in nodes[at]["deps"].as_array().unwrap() {
				if dep["dep_kinds"].as_array().unwrap().iter().any(|kind| kind["kind"].as_str() != Some("dev")) {
					stack.push(dep["pkg"].as_str().unwrap());
				}
			}
		}
		closure.into_iter().map(|at| names[at]).collect::<BTreeSet<_>>()
	};
	let runner = ["rust", "_runner"].concat();
	assert!(
		reached(None).contains(runner.as_str()),
		"the graph was walked: file_host reaches it through leetype_runner today"
	);
	assert!(
		!reached(Some(id("leetype_runner"))).contains(runner.as_str()),
		"file_host reaches {runner} other than through leetype_runner"
	);
}
