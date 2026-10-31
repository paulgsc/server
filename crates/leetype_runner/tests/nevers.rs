//! #381's never #5, structurally: nothing derived from a run may reach the
//! ledger, the sampler or the study nudge. `StudySignal` lives in
//! `study_domain`; the nudge's state and outcomes live in `intervention`,
//! `outcome_repo`, `session_repo` and `engagement_repo`. None of them is in
//! this crate's dependency closure, so no function here can construct a
//! `StudySignal` or write one anywhere — not by review, but because the type
//! is not in scope for the compiler.
//!
//! Read from `cargo metadata`'s resolved graph (normal and build edges; dev
//! edges are tests'), so a dependency added anywhere below this crate that
//! drags one of them in fails here.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const FORBIDDEN: [&str; 5] = ["study_domain", "intervention", "outcome_repo", "session_repo", "engagement_repo"];

#[test]
fn nothing_in_the_runners_dependency_closure_knows_a_study_signal() {
	// Set by cargo for the test it runs: the toolchain's own `cargo`.
	#[allow(clippy::disallowed_methods)]
	let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
	// The graph built for this host. Unfiltered, `--offline` needs every
	// platform's crates downloaded (crossterm's Windows-only one, via envctl),
	// though nothing here builds them.
	let version = std::process::Command::new(&cargo).arg("-vV").output().unwrap();
	let host = String::from_utf8_lossy(&version.stdout)
		.lines()
		.find_map(|line| line.strip_prefix("host: ").map(str::to_owned))
		.unwrap();
	let metadata = std::process::Command::new(cargo)
		.args(["metadata", "--format-version", "1", "--offline", "--filter-platform", &host, "--manifest-path"])
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
	let root = names.iter().find(|(_, name)| **name == "leetype_runner").map(|(id, _)| *id).unwrap();

	let mut closure = BTreeSet::new();
	let mut stack = vec![root];
	while let Some(id) = stack.pop() {
		if !closure.insert(id) {
			continue;
		}
		for dep in nodes[id]["deps"].as_array().unwrap() {
			let normal = dep["dep_kinds"].as_array().unwrap().iter().any(|kind| kind["kind"].as_str() != Some("dev"));
			if normal {
				stack.push(dep["pkg"].as_str().unwrap());
			}
		}
	}
	let reached: BTreeSet<&str> = closure.iter().map(|id| names[id]).collect();
	assert!(reached.contains("leetype_round_repo"), "the closure was walked: {reached:?}");
	for crate_name in FORBIDDEN {
		assert!(!reached.contains(crate_name), "{crate_name} is reachable from leetype_runner");
	}
}
