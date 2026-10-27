mod common;

use playoff_delta::{
	evaluate::{evaluate, EvaluateError, Subject},
	league::Team,
	story::Kind,
};
use serde_json::Value;
use std::collections::BTreeSet;

fn niners() -> Subject {
	Subject {
		team: Team::Sf,
		label: "Invented season".into(),
		// Few ways and rests, so a debug build tests quickly; the job counts
		// `WAYS` and `RESTS`.
		ways: 1_000,
		rests: 64,
	}
}

#[test]
fn every_week_follows_from_the_last() {
	let story = evaluate(&common::invented(7, 17).unwrap(), &niners()).unwrap();
	assert_eq!(story.weeks.len(), 17);
	for pair in story.weeks.windows(2) {
		assert_eq!(pair[1].paths.before, pair[0].paths.after, "week {}", pair[1].week);
		assert_eq!(pair[1].paths.wins_before, pair[0].paths.wins_after);
		assert_eq!(pair[1].share.before, pair[0].share.after);
		// In whatever happens stays in.
		assert!(!pair[0].clinched || pair[1].clinched, "week {}", pair[1].week);
	}
	for w in &story.weeks {
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
		assert!(w.share.spread.0 >= 0.0);
		// A clinch is every way in: a share of one.
		if w.clinched {
			assert!((w.share.after.0 - 1.0).abs() < 1e-12, "week {}", w.week);
		}
		match &w.stake {
			// Next week's spread is never less than the team's own game's part of it.
			Some(stake) => {
				assert!(stake.spread.0 >= stake.own.0 / 2.0 - 1e-12, "week {}: {stake:?}", w.week);
				// A swing needs a game to swing on.
				assert!(stake.opponent.is_some() || stake.own.0 == 0.0, "week {}: {stake:?}", w.week);
			}
			None => assert_eq!(w.pending.after, 0, "week {}", w.week),
		}
	}
	assert!(story.weeks.iter().any(|w| w.share.spread.0 > 0.0), "no week could move the share");
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
	assert_eq!(bye.paths.wins_before, bye.paths.wins_after);
	assert!(bye.share.own.0.abs() < 1e-12, "a bye has no result of its own");
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

/// The keys `story.ts` (contract v3) declares. A key added there is added
/// here, and to `story.rs`, in the same pair of changes.
const CONTRACT: &str = "
version label seasonLength team names conference ways weeks
weeks[].week weeks[].record weeks[].record.w weeks[].record.l
weeks[].game weeks[].game.opponent weeks[].game.won weeks[].game.relation
weeks[].paths weeks[].paths.before weeks[].paths.after weeks[].paths.winsBefore weeks[].paths.winsAfter
weeks[].share weeks[].share.before weeks[].share.after weeks[].share.own weeks[].share.league weeks[].share.spread weeks[].share.exact
weeks[].pending weeks[].pending.before weeks[].pending.after
weeks[].results weeks[].results[].winner weeks[].results[].loser weeks[].results[].kind weeks[].results[].leverage
weeks[].results[].collapse weeks[].results[].collapse.peakWinProbability weeks[].results[].collapse.led
weeks[].next weeks[].next[].home weeks[].next[].away weeks[].next[].rootFor weeks[].next[].swing
weeks[].stake weeks[].stake.opponent weeks[].stake.own weeks[].stake.spread weeks[].stake.exact
weeks[].clinched
";

#[test]
fn serializes_to_the_contract_keys() {
	let story = evaluate(&common::with_byes(7).unwrap(), &niners()).unwrap();
	let mut seen = BTreeSet::new();
	key_paths(&serde_json::to_value(&story).unwrap(), "", &mut seen);
	let allowed: BTreeSet<String> = CONTRACT.split_whitespace().map(String::from).collect();
	let extra: Vec<_> = seen.difference(&allowed).collect();
	assert!(extra.is_empty(), "keys the contract does not have: {extra:?}");
	for required in [
		"weeks[].results[].leverage",
		"weeks[].next[].rootFor",
		"weeks[].share.spread",
		"weeks[].stake.spread",
		"weeks[].clinched",
	] {
		assert!(seen.contains(required), "missing {required}");
	}
}
