mod common;

use playoff_delta::{
	evaluate::{evaluate, EvaluateError, Subject},
	league::Team,
	story::{Claim, DivisionRoute, Kind},
};
use serde_json::Value;
use std::collections::BTreeSet;

fn niners() -> Subject {
	Subject {
		team: Team::Sf,
		rivals: vec![Team::Dal, Team::Sea],
		label: "Invented season".into(),
		// Few ways, so a debug build tests quickly; the job counts `WAYS`.
		ways: 1_000,
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
		assert!(w.results.iter().all(|r| r.kind != Kind::Own || r.moved.is_none()));
		for r in &w.results {
			// A conference team's loss to the other conference never hurts.
			if r.kind == Kind::Exported {
				let sign = if r.winner.conference() == Team::Sf.conference() { -1.0 } else { 1.0 };
				assert!(sign * r.leverage.0 >= 0.0, "week {}: {} beat {}", w.week, r.winner.code(), r.loser.code());
			}
		}
		assert!(w.results.windows(2).all(|p| p[0].leverage.0.abs() >= p[1].leverage.0.abs()));
		assert!(w
			.next
			.iter()
			.all(|n| n.home != Team::Sf && n.away != Team::Sf && (n.root_for == n.home || n.root_for == n.away)));
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
fn a_bye_is_a_week_with_no_game() {
	let story = evaluate(&common::with_byes(7).unwrap(), &common::invented(6, 17).unwrap(), &niners()).unwrap();
	assert_eq!(story.weeks.len(), 18);
	let byes: Vec<_> = story.weeks.iter().filter(|w| w.game.is_none()).collect();
	assert_eq!(byes.len(), 1);
	let bye = byes[0];
	assert!(bye.week == 9 || bye.week == 10);
	assert_eq!(bye.paths.wins_before, bye.paths.wins_after);
	assert!(bye.share.own.0.abs() < 1e-12, "a bye has no result of its own");
	assert!(bye.results.iter().all(|r| r.kind != Kind::Own));
}

#[test]
fn a_boundary_is_a_proof_and_free_teams_come_last() {
	let story = evaluate(&common::with_byes(7).unwrap(), &common::invented(6, 17).unwrap(), &niners()).unwrap();
	let mut boundaries = 0;
	for w in &story.weeks {
		let claim = claim_of(&w.paths.after, w.paths.wins_after);
		let Some(b) = &w.boundary else {
			// Only a claim of "never win again", or none, has no row below it.
			assert!(!matches!(claim, Claim::In { extra_wins, .. } if extra_wins > 0), "week {}: no proof", w.week);
			continue;
		};
		boundaries += 1;
		let Claim::In { wins, extra_wins, .. } = claim else {
			panic!("week {}: a boundary while out", w.week)
		};
		assert!(extra_wins > 0);
		assert_eq!(b.wins, wins + extra_wins - 1);
		if let DivisionRoute::Pigeonhole { teams, games } = &b.division {
			let room: u16 = teams.iter().map(|t| u16::from(t.room)).sum();
			assert!(*games > room, "week {}: {games} games, room for {room}", w.week);
			assert!(teams.iter().all(|t| t.team.division() == Team::Sf.division()));
		}
		if let Some(p) = &b.wild_card.pigeonhole {
			let room: u16 = p.teams.iter().map(|t| u16::from(t.room)).sum();
			assert!(p.games > room && p.must_pass > 0, "week {}", w.week);
		}
	}
	assert!(boundaries > 0, "the invented season never needs a win");
	// Past the rivals and division mates, a team the way lets finish above is
	// listed after every team it holds down.
	let mut frees = 0;
	for p in story.weeks.iter().filter_map(|w| w.path.as_ref()) {
		let others: Vec<_> = p.needs.iter().filter(|n| !n.rival && n.team.division() != Team::Sf.division()).collect();
		frees += others.iter().filter(|n| n.free).count();
		assert!(others.windows(2).all(|w| !w[0].free || w[1].free), "{others:?}");
	}
	assert!(frees > 0, "no path lets a team finish above");
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

/// The keys `story.ts` (contract v2) declares. A key added there is added
/// here, and to `story.rs`, in the same pair of changes.
const CONTRACT: &str = "
version label seasonLength team rivals names conference ways weeks
weeks[].week weeks[].record weeks[].record.w weeks[].record.l
weeks[].game weeks[].game.opponent weeks[].game.won weeks[].game.relation
weeks[].paths weeks[].paths.before weeks[].paths.after weeks[].paths.winsBefore weeks[].paths.winsAfter
weeks[].share weeks[].share.before weeks[].share.after weeks[].share.own weeks[].share.league weeks[].share.exact
weeks[].pending weeks[].pending.before weeks[].pending.after
weeks[].results weeks[].results[].winner weeks[].results[].loser weeks[].results[].kind weeks[].results[].leverage
weeks[].results[].moved weeks[].results[].collapse weeks[].results[].collapse.peakWinProbability weeks[].results[].collapse.led
weeks[].next weeks[].next[].home weeks[].next[].away weeks[].next[].rootFor weeks[].next[].swing
weeks[].path weeks[].path.extraWins weeks[].path.seed weeks[].path.needs
weeks[].path.needs[].team weeks[].path.needs[].rest weeks[].path.needs[].rest.w weeks[].path.needs[].rest.l
weeks[].path.needs[].rival weeks[].path.needs[].free
weeks[].boundary weeks[].boundary.wins weeks[].boundary.division weeks[].boundary.division.kind weeks[].boundary.division.team
weeks[].boundary.division.teams weeks[].boundary.division.teams[].team weeks[].boundary.division.teams[].wins
weeks[].boundary.division.teams[].room weeks[].boundary.division.games
weeks[].boundary.wildCard weeks[].boundary.wildCard.above weeks[].boundary.wildCard.pigeonhole
weeks[].boundary.wildCard.pigeonhole.teams weeks[].boundary.wildCard.pigeonhole.teams[].team
weeks[].boundary.wildCard.pigeonhole.teams[].wins weeks[].boundary.wildCard.pigeonhole.teams[].room
weeks[].boundary.wildCard.pigeonhole.games weeks[].boundary.wildCard.pigeonhole.mustPass
weeks[].boundary.wildCard.cases
weeks[].rivals weeks[].rivals[].team weeks[].rivals[].game weeks[].rivals[].game.opponent weeks[].rivals[].game.won
weeks[].rivals[].share weeks[].rivals[].share.before weeks[].rivals[].share.after
weeks[].collisions weeks[].collisions[].kind weeks[].collisions[].home weeks[].collisions[].away weeks[].collisions[].winner
weeks[].collisions[].rival weeks[].collisions[].worstForRival weeks[].collisions[].bestForYou
weeks[].points weeks[].points.week weeks[].points.week.scored weeks[].points.week.possible
weeks[].points.season weeks[].points.season.scored weeks[].points.season.possible
weeks[].benchmark weeks[].benchmark.lowestBar weeks[].benchmark.lowestBar.team weeks[].benchmark.lowestBar.wins weeks[].benchmark.lowestBar.seed
";

const CLAIMS: [&str; 3] = ["weeks[].rivals[].before", "weeks[].rivals[].after", "weeks[].benchmark.lastSeason"];

#[test]
fn serializes_to_the_contract_keys() {
	let story = evaluate(&common::with_byes(7).unwrap(), &common::invented(6, 17).unwrap(), &niners()).unwrap();
	let mut seen = BTreeSet::new();
	key_paths(&serde_json::to_value(&story).unwrap(), "", &mut seen);
	let mut allowed: BTreeSet<String> = CONTRACT.split_whitespace().map(String::from).collect();
	for c in CLAIMS {
		allowed.insert(c.into());
		for k in ["in", "wins", "extraWins", "seed"] {
			allowed.insert(String::from(c) + "." + k);
		}
	}
	let extra: Vec<_> = seen.difference(&allowed).collect();
	assert!(extra.is_empty(), "keys the contract does not have: {extra:?}");
	for required in [
		"weeks[].path.needs[].free",
		"weeks[].results[].leverage",
		"weeks[].next[].rootFor",
		"weeks[].boundary.division.teams[].room",
		"weeks[].rivals[].share.after",
		"weeks[].benchmark.lastSeason.in",
	] {
		assert!(seen.contains(required), "missing {required}");
	}
}
