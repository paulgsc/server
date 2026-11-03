mod common;

use playoff_delta::{
	evaluate::{evaluate, EvaluateError, Subject},
	league::Team,
	story::{Cohort, Kind, Poles},
};
use serde_json::Value;
use std::collections::BTreeSet;

fn niners() -> Subject {
	Subject {
		team: Team::Sf,
		label: "Invented season".into(),
	}
}

/// A seed as a rung number, out last.
fn rung(seed: Option<u8>) -> u8 {
	seed.unwrap_or(8)
}

#[test]
fn every_week_follows_from_the_last() {
	let story = evaluate(&common::invented(7, 17).unwrap(), &niners()).unwrap();
	assert_eq!(story.weeks.len(), 17);
	// PD4: the poles only narrow.
	for w in &story.weeks {
		let (b, a): (&Poles, &Poles) = (&w.poles.before, &w.poles.after);
		for (gb, ga) in [(&b.division, &a.division), (&b.conference, &a.conference)] {
			assert!(ga.floor >= gb.floor && ga.ceiling.lose <= gb.ceiling.lose, "week {}", w.week);
		}
		assert!(a.us.low >= b.us.low && a.us.high <= b.us.high, "week {}", w.week);
		assert!(rung(a.seeds.best) >= rung(b.seeds.best) && rung(a.seeds.worst) <= rung(b.seeds.worst), "week {}", w.week);
	}
	for p in story.weeks.iter().flat_map(|w| [&w.poles.before, &w.poles.after]) {
		for (g, size) in [(&p.division, 3), (&p.conference, 15)] {
			assert!(g.floor <= g.ceiling.win && g.ceiling.win <= g.ceiling.lose);
			assert!(!g.bloc.teams.is_empty() && g.bloc.teams.len() <= size);
		}
		assert!(p.division.bloc.teams.iter().all(|t| t.division() == Team::Sf.division() && *t != Team::Sf));
		assert!(p.held >= p.conference.floor && p.held >= p.division.floor);
		assert!(rung(p.seeds.best) <= rung(p.seeds.worst));
	}
	assert_eq!(story.rivals, [Team::Lar, Team::Sea, Team::Ari]);
}

#[test]
fn each_result_says_which_pole_it_moved() {
	let season = common::invented(7, 12).unwrap();
	let story = evaluate(&season, &niners()).unwrap();
	let rivals = &story.rivals;
	for w in &story.weeks {
		// Kickoff order: the invented season's slots run backwards.
		let slots: Vec<u64> = w
			.results
			.iter()
			.map(|r| season.week(w.week).find(|g| g.home == r.home && g.away == r.away).and_then(|g| g.kickoff).unwrap())
			.collect();
		assert!(slots.windows(2).all(|p| p[0] <= p[1]), "week {}: {slots:?}", w.week);
		for r in &w.results {
			assert!(r.winner == r.home || r.winner == r.away);
			let ours = r.home == Team::Sf || r.away == Team::Sf;
			assert_eq!(r.kind == Kind::Own, ours);
			let touches = ours || rivals.contains(&r.home) || rivals.contains(&r.away);
			assert_eq!(r.division.is_some(), touches, "week {}: {r:?}", w.week);
			for (m, both_in) in [
				(Some(r.conference), r.kind == Kind::Inside),
				(r.division, rivals.contains(&r.home) && rivals.contains(&r.away)),
			] {
				let Some(m) = m else { continue };
				if ours {
					let won = r.winner == Team::Sf;
					assert_eq!(m.cohort, if won { Cohort::Best } else { Cohort::Worst }, "week {}: {r:?}", w.week);
				} else if both_in {
					assert_eq!(m.cohort, Cohort::Forced, "week {}: {r:?}", w.week);
				} else {
					// The cohort is the sign of what moved.
					let moved = (m.floor.0.signum(), m.ceiling.0.signum());
					let cohort = match moved {
						(0, 0) => Cohort::Idle,
						(a, b) if a > 0 || b > 0 => Cohort::Worst,
						_ => Cohort::Best,
					};
					assert_eq!(m.cohort, cohort, "week {}: {r:?}", w.week);
				}
				// A loss to a team outside the group never raises it; a win
				// never lowers it.
				if !ours && !both_in {
					assert!(m.floor.0 * m.ceiling.0 >= 0, "week {}: {r:?}", w.week);
				}
			}
		}
	}
	assert!(story.weeks.iter().flat_map(|w| &w.results).any(|r| r.conference.cohort == Cohort::Worst));
	assert!(story.weeks.iter().flat_map(|w| &w.results).any(|r| r.conference.cohort == Cohort::Best));
}

#[test]
fn the_same_season_tells_the_same_story() {
	let season = common::invented(11, 9).unwrap();
	let a = serde_json::to_value(evaluate(&season, &niners()).unwrap()).unwrap();
	let b = serde_json::to_value(evaluate(&season, &niners()).unwrap()).unwrap();
	assert_eq!(a, b);
	assert_eq!(a["weeks"].as_array().map(Vec::len), Some(9));
}

#[test]
fn a_bye_is_a_week_with_no_game() {
	let story = evaluate(&common::with_byes(7).unwrap(), &niners()).unwrap();
	assert_eq!(story.weeks.len(), 18);
	let byes: Vec<_> = story.weeks.iter().filter(|w| w.game.is_none()).collect();
	assert_eq!(byes.len(), 1);
	let bye = byes[0];
	assert!(bye.week == 9 || bye.week == 10);
	assert_eq!(bye.poles.before.us, bye.poles.after.us, "a bye has no result of its own");
	assert!(bye.results.iter().all(|r| r.kind != Kind::Own));
}

#[test]
fn refuses_what_it_cannot_tell() {
	assert_eq!(evaluate(&common::invented(7, 0).unwrap(), &niners()).unwrap_err(), EvaluateError::NothingPlayed);
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

/// The keys `story.ts` (contract v6) declares. A key added there is added
/// here, and to `story.rs`, in the same pair of changes.
const CONTRACT: &str = "
version label seasonLength team names conference rivals weeks
weeks[].week weeks[].record weeks[].record.w weeks[].record.l
weeks[].game weeks[].game.opponent weeks[].game.won weeks[].game.relation
weeks[].pending weeks[].pending.before weeks[].pending.after
weeks[].poles weeks[].poles.before weeks[].poles.after
weeks[].poles.before.us weeks[].poles.before.us.low weeks[].poles.before.us.high
weeks[].poles.before.division weeks[].poles.before.division.floor
weeks[].poles.before.division.bloc weeks[].poles.before.division.bloc.teams
weeks[].poles.before.division.bloc.banked weeks[].poles.before.division.bloc.games
weeks[].poles.before.division.ceiling weeks[].poles.before.division.ceiling.lose
weeks[].poles.before.division.ceiling.win weeks[].poles.before.division.ceiling.team
weeks[].poles.before.conference weeks[].poles.before.conference.floor
weeks[].poles.before.conference.bloc weeks[].poles.before.conference.bloc.teams
weeks[].poles.before.conference.bloc.banked weeks[].poles.before.conference.bloc.games
weeks[].poles.before.conference.ceiling weeks[].poles.before.conference.ceiling.lose
weeks[].poles.before.conference.ceiling.win weeks[].poles.before.conference.ceiling.team
weeks[].poles.before.held weeks[].poles.before.seeds weeks[].poles.before.seeds.best weeks[].poles.before.seeds.worst
weeks[].results weeks[].results[].home weeks[].results[].away weeks[].results[].winner weeks[].results[].kind
weeks[].results[].division weeks[].results[].division.cohort weeks[].results[].division.floor weeks[].results[].division.ceiling
weeks[].results[].conference weeks[].results[].conference.cohort weeks[].results[].conference.floor
weeks[].results[].conference.ceiling
";

#[test]
fn serializes_to_the_contract_keys() {
	let story = evaluate(&common::with_byes(7).unwrap(), &niners()).unwrap();
	let mut seen = BTreeSet::new();
	key_paths(&serde_json::to_value(&story).unwrap(), "", &mut seen);
	// `after` has the same shape as `before`.
	let seen: BTreeSet<String> = seen.into_iter().map(|k| k.replace("poles.after.", "poles.before.")).collect();
	let allowed: BTreeSet<String> = CONTRACT.split_whitespace().map(String::from).collect();
	let extra: Vec<_> = seen.difference(&allowed).collect();
	assert!(extra.is_empty(), "keys the contract does not have: {extra:?}");
	let missing: Vec<_> = allowed.difference(&seen).collect();
	assert!(missing.is_empty(), "contract keys never written: {missing:?}");
}
