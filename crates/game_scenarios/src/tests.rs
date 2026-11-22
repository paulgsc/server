use super::{catalog, Scenario, CATALOG_VERSION};
use std::{
	collections::{BTreeMap, HashSet},
	path::PathBuf,
};

fn is_slug(s: &str) -> bool {
	!s.is_empty() && !s.starts_with('-') && !s.ends_with('-') && !s.contains("--") && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

#[test]
fn all_lists_every_scenario_in_declaration_order() {
	for (i, s) in Scenario::ALL.into_iter().enumerate() {
		assert_eq!(s as usize, i, "{s:?} is out of order in Scenario::ALL");
	}
	assert_eq!(
		Scenario::ALL.len(),
		Scenario::OnsideKick as usize + 1,
		"a scenario was appended after OnsideKick but not to Scenario::ALL"
	);
}

#[test]
fn ids_are_unique_dotted_and_carry_their_phase() {
	let mut seen = HashSet::new();
	for s in Scenario::ALL {
		let e = s.entry();
		assert!(seen.insert(e.id), "{} is used twice", e.id);
		let (head, tail) = e.id.split_once('.').unwrap();
		assert_eq!(serde_json::to_value(e.phase).unwrap(), head, "{} does not start with its phase", e.id);
		assert!(is_slug(tail), "{} is not `<phase>.<kebab-case>`", e.id);
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
	// The whole set but its number: a version may change only a dimension
	// (v3 adds the claim algebra and leaves the scenarios as they were).
	let unnumbered = |set: &serde_json::Value| {
		let mut set = set.clone();
		set.as_object_mut().unwrap().remove("version");
		set
	};
	for (version, old) in files.range(..CATALOG_VERSION) {
		assert_ne!(
			unnumbered(old),
			unnumbered(&current),
			"v{version} and v{CATALOG_VERSION} are the same set: a version is only bumped for a change"
		);
	}
}

#[test]
fn a_half_and_a_size_come_from_their_numbers() {
	use super::{Half, Kind, Size};
	assert_eq!(
		[0, 1, 2, 3, 4, 5, 6].map(Half::of_period),
		[
			None,
			Some(Half::First),
			Some(Half::First),
			Some(Half::Second),
			Some(Half::Second),
			Some(Half::Overtime),
			Some(Half::Overtime)
		]
	);
	assert_eq!(
		[19, 20, 39, 40, 59, 60, 99].map(Size::of_yards),
		[
			None,
			Some(Size::Big),
			Some(Size::Big),
			Some(Size::Huge),
			Some(Size::Huge),
			Some(Size::Memorable),
			Some(Size::Memorable)
		]
	);
	assert_eq!(
		[25, 39, 40].map(|y| Kind::KickReturn.size(y)),
		[None, None, Some(Size::Huge)],
		"an ordinary kick return is not big"
	);
	assert_eq!(Kind::PuntReturn.size(25), Some(Size::Big));
}

#[test]
fn the_claim_algebra_is_closed_and_each_side_is_one_shapes() {
	use super::{Better, Deadline, Measure, Shape, Side, Unit, Verdict};
	// Each `ALL` lists every variant once, in declaration order. A match with
	// no wildcard fails to compile on a new variant until it is placed here;
	// the assert then fails until `ALL` lists it at that place.
	let place = |measure: Measure| match measure {
		Measure::PassRush => 0,
		Measure::ThirdDown => 1,
		Measure::RedZone => 2,
		Measure::FourthDownGo => 3,
		Measure::TwoMinute => 4,
		Measure::TimeoutsAtHalf => 5,
		Measure::Takeaways => 6,
		Measure::ExplosivesAllowed => 7,
	};
	assert_eq!(Measure::ALL.map(place), [0, 1, 2, 3, 4, 5, 6, 7]);
	let shape = |shape: Shape| match shape {
		Shape::Rank => 0,
		Shape::VsLeague => 1,
		Shape::VsTeam => 2,
		Shape::Trend => 3,
	};
	assert_eq!(Shape::ALL.map(shape), [0, 1, 2, 3]);
	let deadline = |deadline: Deadline| match deadline {
		Deadline::AfterGames => 0,
		Deadline::ByWeek => 1,
		Deadline::SeasonEnd => 2,
	};
	assert_eq!(Deadline::ALL.map(deadline), [0, 1, 2]);
	let verdict = |verdict: Verdict| match verdict {
		Verdict::Holding => 0,
		Verdict::AgainstSoFar => 1,
		Verdict::TooEarly => 2,
		Verdict::SettledRight => 3,
		Verdict::SettledWrong => 4,
		Verdict::Withdrawn => 5,
	};
	assert_eq!(Verdict::ALL.map(verdict), [0, 1, 2, 3, 4, 5]);
	// A shape's two sides differ, the higher value first, and every side is
	// some shape's. vs-league and vs-team share theirs, so a claim stores its
	// shape beside its side.
	let side = |side: Side| match side {
		Side::Highest | Side::Lowest => Shape::Rank,
		Side::Above | Side::Below => Shape::VsLeague,
		Side::Rising | Side::Falling => Shape::Trend,
	};
	for shape in Shape::ALL {
		let [higher, lower] = shape.sides();
		assert_ne!(higher, lower, "{shape:?}'s sides are one side");
		assert_eq!(side(higher).sides(), shape.sides(), "{shape:?}'s sides are another shape's");
	}
	assert_eq!(
		[Side::Highest, Side::Lowest, Side::Above, Side::Below, Side::Rising, Side::Falling].map(|each| Shape::ALL.iter().any(|shape| shape.sides().contains(&each))),
		[true; 6],
		"a side no shape has"
	);
	assert_eq!(Shape::VsTeam.sides(), Shape::VsLeague.sides(), "a named team is compared as the league is");
	// The prototype's reading: a style has no good direction, and allowing
	// big plays is good when fewer.
	assert_eq!(Measure::FourthDownGo.better(), None);
	assert_eq!(Measure::ExplosivesAllowed.better(), Some(Better::Lower));
	assert!(Measure::ALL.iter().all(|m| m.unit() != Unit::SpecialTeams));
}
