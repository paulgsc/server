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
		// A clinch is every way in: a share of one.
		if w.clinched {
			assert!((w.share.after.0 - 1.0).abs() < 1e-12, "week {}", w.week);
		}
		// A swing needs a game to swing on.
		if let Some(stake) = &w.stake {
			assert!(stake.opponent.is_some() || stake.own.0 == 0.0, "week {}: {stake:?}", w.week);
		}
	}
	assert!(story.weeks.iter().any(|w| w.share.spread.0 > 0.0), "no week could move the share");
}

#[test]
fn a_week_s_ways_add_up() {
	let season = common::invented(7, 12).unwrap();
	let story = evaluate(&season, &niners()).unwrap();
	for w in &story.weeks {
		let ways = &w.ways;
		let games = ways.games.len();
		assert!(ways.worst.0 <= ways.actual.0 && ways.actual.0 <= ways.best.0, "week {}", w.week);
		// The road runs from the best way to the actual one and never rises;
		// the mean of the ways still possible ends at the actual share.
		assert_eq!(ways.road.len(), games + 1);
		assert!((ways.road[0].0 - ways.best.0).abs() < 1e-12);
		assert!((ways.road[games].0 - ways.actual.0).abs() < 1e-12);
		assert!((ways.mean[games].0 - ways.actual.0).abs() < 1e-12);
		assert!(ways.road.windows(2).all(|p| p[1].0 <= p[0].0), "week {}", w.week);
		// The strip halves at each result.
		for (p, row) in ways.strip.iter().enumerate() {
			assert_eq!(row.iter().sum::<u32>(), (1 << games) >> p, "week {} point {p}", w.week);
		}
		// Every piece names a side of its game.
		for g in &ways.games {
			assert!(g.piece.want == g.home || g.piece.want == g.away);
			assert!(g.winner == g.home || g.winner == g.away);
		}
		match (&w.game, &ways.given) {
			(None, given) => assert!(given.is_none(), "a bye holds no result"),
			(Some(_), Some(given)) => {
				assert!(given.best.0 <= ways.best.0 + 1e-12);
				assert_eq!(given.pieces.len(), games);
				let own = ways.games.iter().position(|g| g.kind == Kind::Own).unwrap();
				assert_eq!(given.pieces[own].want, ways.games[own].winner);
				assert!(given.pieces[own].price.0.abs() < 1e-12);
			}
			(Some(_), None) => panic!("week {}: a played week holds its result", w.week),
		}
		// Kickoff order: the invented season's slots run backwards.
		let slots: Vec<u64> = ways
			.games
			.iter()
			.map(|g| season.week(w.week).find(|s| s.home == g.home && s.away == g.away).and_then(|s| s.kickoff).unwrap())
			.collect();
		assert!(slots.windows(2).all(|p| p[0] <= p[1]), "week {}: {slots:?}", w.week);
		// The podium places only eligible teams, 1 to 3, best rank first.
		let p = &w.podium;
		assert!(p.medals.len() >= usize::from(p.eligible.min(3)));
		assert!(p.medals.windows(2).all(|m| m[0].place <= m[1].place && m[0].rank.0 >= m[1].rank.0));
		assert!(p.medals.iter().all(|m| (1..=3).contains(&m.place) && m.pieces.len() <= 3));
		if let Some(first) = p.medals.first() {
			assert_eq!(first.place, 1);
		}
		if let Some(place) = p.place {
			assert!(place >= 1 && place <= p.eligible);
			// Our rank is the one our ways give.
			if let Some(us) = p.medals.iter().find(|m| m.team == Team::Sf) {
				assert!((us.rank.0 - ways.rank.0).abs() < 1e-12);
			}
		}
		if let Some(stake) = &w.stake {
			assert!(stake.pieces.iter().all(|g| g.piece.want == g.home || g.piece.want == g.away));
		}
	}
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

/// The keys `story.ts` (contract v4) declares. A key added there is added
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
weeks[].stake.pieces weeks[].stake.pieces[].home weeks[].stake.pieces[].away
weeks[].stake.pieces[].want weeks[].stake.pieces[].price weeks[].stake.pieces[].error
weeks[].clinched
weeks[].ways weeks[].ways.games weeks[].ways.games[].home weeks[].ways.games[].away weeks[].ways.games[].kind
weeks[].ways.games[].winner weeks[].ways.games[].want weeks[].ways.games[].price weeks[].ways.games[].error
weeks[].ways.best weeks[].ways.worst weeks[].ways.actual weeks[].ways.rank
weeks[].ways.given weeks[].ways.given.best weeks[].ways.given.rank weeks[].ways.given.pieces
weeks[].ways.given.pieces[].want weeks[].ways.given.pieces[].price weeks[].ways.given.pieces[].error
weeks[].ways.road weeks[].ways.mean weeks[].ways.strip weeks[].ways.exact
weeks[].podium weeks[].podium.medals weeks[].podium.medals[].team weeks[].podium.medals[].place
weeks[].podium.medals[].rank weeks[].podium.medals[].best weeks[].podium.medals[].worst weeks[].podium.medals[].actual
weeks[].podium.medals[].game weeks[].podium.medals[].pieces weeks[].podium.medals[].pieces[].home
weeks[].podium.medals[].pieces[].away weeks[].podium.medals[].pieces[].want weeks[].podium.medals[].pieces[].price
weeks[].podium.medals[].pieces[].made weeks[].podium.place weeks[].podium.eligible
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
		"weeks[].stake.spread",
		"weeks[].stake.pieces[].want",
		"weeks[].ways.games[].price",
		"weeks[].ways.given.pieces[].price",
		"weeks[].podium.medals[].pieces[].made",
	] {
		assert!(seen.contains(required), "missing {required}");
	}
}
