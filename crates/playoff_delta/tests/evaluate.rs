mod common;

use playoff_delta::{
	evaluate::{evaluate, EvaluateError, Subject},
	league::Team,
	story::Claim,
};
use serde_json::Value;
use std::collections::BTreeSet;

fn niners() -> Subject {
	Subject {
		team: Team::Sf,
		rivals: vec![Team::Dal, Team::Sea],
		label: "Invented season".into(),
	}
}

/// The claim a paths row implies, as the client's `claimOf` reads it.
fn claim_of(paths: &[Option<u8>], wins: u8) -> Claim {
	let first = paths.iter().position(Option::is_some);
	Claim::new(wins, first.and_then(|x| Some((u8::try_from(x).ok()?, paths[x]?))))
}

#[test]
fn every_week_follows_from_the_last() {
	let story = evaluate(&common::invented(7, 17).unwrap(), &common::invented(6, 17).unwrap(), &niners()).unwrap();
	assert_eq!(story.weeks.len(), 17);
	let mut season = (0, 0);
	for pair in story.weeks.windows(2) {
		assert_eq!(pair[1].paths.before, pair[0].paths.after, "week {}", pair[1].week);
		assert_eq!(pair[1].paths.wins_before, pair[0].paths.wins_after);
	}
	for w in &story.weeks {
		// The path is the after row's claim, and exists exactly when it does.
		match (claim_of(&w.paths.after, w.paths.wins_after), &w.path) {
			(Claim::In { extra_wins, seed, .. }, Some(p)) => assert_eq!((p.extra_wins, p.seed), (extra_wins, seed)),
			(Claim::Out { .. }, None) => {}
			(c, p) => panic!("week {}: claim {c:?}, path {p:?}", w.week),
		}
		// Your own game never "moves" anything: the game slide is its effect.
		assert!(w.templates.division.iter().all(|r| r.opponent != Team::Sf || r.moved.is_none()));
		for r in &w.rivals {
			assert!(r.worst.rank() >= r.after.rank(), "week {}: {}'s worst week beats its real one", w.week, r.team.code());
		}
		season = (season.0 + w.points.week.scored, season.1 + w.points.week.possible);
		assert_eq!((w.points.season.scored, w.points.season.possible), season);
		assert!(w.points.week.scored <= w.points.week.possible);
	}
}

#[test]
fn the_same_season_tells_the_same_story() {
	let (season, previous) = (common::invented(11, 9).unwrap(), common::invented(10, 17).unwrap());
	let a = serde_json::to_value(evaluate(&season, &previous, &niners()).unwrap()).unwrap();
	let b = serde_json::to_value(evaluate(&season, &previous, &niners()).unwrap()).unwrap();
	assert_eq!(a, b);
	assert_eq!(a["weeks"].as_array().map(Vec::len), Some(9));
}

#[test]
fn a_bye_is_not_a_slide_but_its_results_count() {
	let story = evaluate(&common::with_byes(7).unwrap(), &common::invented(6, 17).unwrap(), &niners()).unwrap();
	assert_eq!(story.weeks.len(), 17);
	let weeks: Vec<u8> = story.weeks.iter().map(|w| w.week).collect();
	let bye = (1..=18).find(|w| !weeks.contains(w)).unwrap();
	assert!(bye == 9 || bye == 10);
	assert_eq!(story.weeks.last().map(|w| w.week), Some(18));
}

#[test]
fn refuses_what_it_cannot_tell() {
	let previous = common::invented(6, 17).unwrap();
	assert_eq!(evaluate(&common::invented(7, 0).unwrap(), &previous, &niners()).unwrap_err(), EvaluateError::NothingPlayed);
	let mut subject = niners();
	subject.rivals.push(Team::Sf);
	assert_eq!(evaluate(&common::invented(7, 3).unwrap(), &previous, &subject).unwrap_err(), EvaluateError::SelfRival);
}

/// Every key path in `value`, arrays folded into `[]`, `names`' keys skipped.
fn key_paths(value: &Value, at: &str, out: &mut BTreeSet<String>) {
	match value {
		Value::Object(map) => {
			for (k, v) in map {
				let path = if at.is_empty() { k.clone() } else { String::from(at) + "." + k };
				out.insert(path.clone());
				if k != "names" {
					key_paths(v, &path, out);
				}
			}
		}
		Value::Array(items) => {
			for v in items {
				key_paths(v, &(String::from(at) + "[]"), out);
			}
		}
		_ => {}
	}
}

/// The keys `story.ts` (contract v1) declares. A key added there is added
/// here, and to `story.rs`, in the same pair of changes.
const CONTRACT: &str = "
version label seasonLength team rivals names conference weeks
weeks[].week weeks[].record weeks[].record.w weeks[].record.l
weeks[].game weeks[].game.opponent weeks[].game.won weeks[].game.relation
weeks[].paths weeks[].paths.before weeks[].paths.after weeks[].paths.winsBefore weeks[].paths.winsAfter
weeks[].templates weeks[].path weeks[].path.extraWins weeks[].path.seed weeks[].path.needs
weeks[].path.needs[].team weeks[].path.needs[].rest weeks[].path.needs[].rest.w weeks[].path.needs[].rest.l weeks[].path.needs[].rival
weeks[].rivals weeks[].rivals[].team weeks[].rivals[].game weeks[].rivals[].game.opponent weeks[].rivals[].game.won
weeks[].collisions weeks[].collisions[].kind weeks[].collisions[].home weeks[].collisions[].away weeks[].collisions[].winner
weeks[].collisions[].rival weeks[].collisions[].worstForRival weeks[].collisions[].bestForYou
weeks[].points weeks[].points.week weeks[].points.week.scored weeks[].points.week.possible
weeks[].points.season weeks[].points.season.scored weeks[].points.season.possible
weeks[].benchmark weeks[].benchmark.lowestBar weeks[].benchmark.lowestBar.team weeks[].benchmark.lowestBar.wins weeks[].benchmark.lowestBar.seed
";

const CLAIMS: [&str; 4] = [
	"weeks[].rivals[].before",
	"weeks[].rivals[].after",
	"weeks[].rivals[].worst",
	"weeks[].benchmark.lastSeason",
];
const RESULTS: [&str; 6] = ["division", "rivals", "above", "exported", "spoilers", "forced"];

#[test]
fn serializes_to_the_contract_keys() {
	let story = evaluate(&common::invented(7, 17).unwrap(), &common::invented(6, 17).unwrap(), &niners()).unwrap();
	let mut seen = BTreeSet::new();
	key_paths(&serde_json::to_value(&story).unwrap(), "", &mut seen);
	let mut allowed: BTreeSet<String> = CONTRACT.split_whitespace().map(String::from).collect();
	for c in CLAIMS {
		allowed.insert(c.into());
		for k in ["in", "wins", "extraWins", "seed"] {
			allowed.insert(String::from(c) + "." + k);
		}
	}
	for t in RESULTS {
		let at = String::from("weeks[].templates.") + t;
		allowed.insert(at.clone());
		for k in ["team", "opponent", "won", "yourWay", "moved", "collapse", "collapse.peakWinProbability", "collapse.led"] {
			allowed.insert(at.clone() + "[]." + k);
		}
	}
	let extra: Vec<_> = seen.difference(&allowed).collect();
	assert!(extra.is_empty(), "keys the contract does not have: {extra:?}");
	for required in [
		"weeks[].path.needs[].rest.l",
		"weeks[].rivals[].worst.extraWins",
		"weeks[].templates.forced[].yourWay",
		"weeks[].benchmark.lastSeason.in",
	] {
		assert!(seen.contains(required), "missing {required}");
	}
}
