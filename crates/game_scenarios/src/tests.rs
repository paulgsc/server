use super::{catalog, Phase, Scenario, CATALOG_VERSION};
use std::{
	collections::{BTreeMap, HashSet},
	path::PathBuf,
};

fn is_slug(s: &str) -> bool {
	!s.is_empty() && !s.starts_with('-') && !s.ends_with('-') && !s.contains("--") && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

const fn prefix(phase: Phase) -> &'static str {
	match phase {
		Phase::Pre => "pre",
		Phase::In => "in",
		Phase::Post => "post",
		Phase::You => "you",
	}
}

#[test]
fn all_lists_every_scenario_in_declaration_order() {
	for (i, s) in Scenario::ALL.into_iter().enumerate() {
		assert_eq!(s as usize, i, "{s:?} is out of order in Scenario::ALL");
	}
	assert_eq!(
		Scenario::ALL.len(),
		Scenario::TunedInLate as usize + 1,
		"a scenario was appended after TunedInLate but not to Scenario::ALL"
	);
}

#[test]
fn ids_are_unique_dotted_and_carry_their_phase() {
	let mut seen = HashSet::new();
	for s in Scenario::ALL {
		let e = s.entry();
		assert!(seen.insert(e.id), "{} is used twice", e.id);
		let (head, tail) = e.id.split_once('.').unwrap();
		assert_eq!(head, prefix(e.phase), "{} does not start with its phase", e.id);
		assert!(is_slug(tail), "{} is not `<phase>.<kebab-case>`", e.id);
		assert_eq!(Scenario::from_id(e.id), Some(s));
	}
	assert_eq!(Scenario::from_id("in.nothing"), None);
}

#[test]
fn every_scenario_has_words_and_outcomes() {
	for e in catalog().scenarios {
		assert!(!e.name.is_empty() && !e.covers.is_empty(), "{} has no words", e.id);
		assert!(!e.outcomes.is_empty(), "{} has no outcomes", e.id);
		let mut seen = HashSet::new();
		for o in e.outcomes {
			assert!(is_slug(o), "{}: outcome {o:?} is not kebab-case", e.id);
			assert!(seen.insert(o), "{}: outcome {o} is listed twice", e.id);
		}
	}
}

fn testdata() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

/// Every released version's file, by version.
fn released() -> BTreeMap<u32, serde_json::Value> {
	std::fs::read_dir(testdata())
		.unwrap()
		.map(|entry| entry.unwrap().file_name().into_string().unwrap())
		.filter_map(|name| {
			let version = name.strip_prefix("catalog.v")?.strip_suffix(".json")?.parse().ok()?;
			let text = std::fs::read_to_string(testdata().join(&name)).unwrap();
			Some((version, serde_json::from_str(&text).unwrap()))
		})
		.collect()
}

/// A change to the set without a new version, or a version without its file,
/// fails here. Earlier versions' files must all still be there: a bite said
/// under one is read against it.
#[test]
fn the_set_matches_its_released_file() {
	let files = released();
	let versions: Vec<u32> = files.keys().copied().collect();
	assert_eq!(
		versions,
		(1..=CATALOG_VERSION).collect::<Vec<_>>(),
		"testdata/ must hold catalog.v1.json through catalog.v{CATALOG_VERSION}.json, and nothing past it: released versions are kept. \
		 Write the current one with `cargo run -q --bin dump-game-scenarios` (see the README)"
	);
	let current = serde_json::to_value(catalog()).unwrap();
	assert_eq!(
		files[&CATALOG_VERSION], current,
		"the set differs from testdata/catalog.v{CATALOG_VERSION}.json. A released version never changes: bump CATALOG_VERSION and add the next file (see the README)"
	);
	for (version, old) in files.range(..CATALOG_VERSION) {
		assert_ne!(*old, current, "v{version} and v{CATALOG_VERSION} are the same set: a version is only bumped for a change");
	}
}
