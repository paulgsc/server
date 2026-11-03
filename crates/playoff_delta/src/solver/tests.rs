//! The solver against brute force: every outcome of a dozen pending games,
//! seeded by the rule itself (PD1: a tie goes to the team asked about), with
//! banked ties in the standings so half-win room is exercised.

use super::*;
use crate::{
	rank::seed as seed_of,
	table::Pending,
	testing::{random_case, Lcg},
};

/// best[team][extra] over every outcome of the pending games (no ties).
fn brute_force(records: &[Record; 32], pending: &[Pending]) -> Vec<Vec<Option<u8>>> {
	let mut best = vec![vec![None; pending.len() + 1]; 32];
	for outcome in 0u32..1 << pending.len() {
		let mut points = records.map(Record::points);
		let mut wins = [0usize; 32];
		for (i, p) in pending.iter().enumerate() {
			let w = if outcome & (1 << i) == 0 { p.home } else { p.away };
			points[w.index()] += 2;
			wins[w.index()] += 1;
		}
		for t in Team::ALL {
			if let Some(seed) = seed_of(&points, t) {
				let slot = &mut best[t.index()][wins[t.index()]];
				*slot = Some(slot.map_or(seed, |s: u8| s.min(seed)));
			}
		}
	}
	best
}

#[test]
fn matches_brute_force_and_every_scenario_is_real() {
	let mut rng = Lcg(0x5eed);
	for _ in 0..30 {
		let (records, pending) = random_case(&mut rng, 11);
		let oracle = brute_force(&records, &pending);
		let table = Table::from_parts(records, pending.clone());
		for team in Team::ALL {
			for extra in 0..=table.remaining(team) {
				let got = best(&table, team, extra);
				assert_eq!(got.as_ref().map(|s| s.seed), oracle[team.index()][usize::from(extra)], "{} +{extra}", team.code());
				if let Some(s) = got {
					assert_eq!(s.rest[team.index()].w, extra);
					assert!(s.rest.iter().all(|r| r.t == 0), "{} +{extra}: a remaining game was tied", team.code());
					let mut points = records.map(Record::points);
					for t in Team::ALL {
						points[t.index()] += s.rest[t.index()].points();
					}
					assert_eq!(seed_of(&points, team), Some(s.seed), "{} +{extra}: the scenario reaches its seed", team.code());
				}
			}
		}
	}
}

#[test]
fn a_banked_tie_is_half_a_win() {
	let mut records = [Record { w: 8, l: 8, t: 0 }; 32];
	// The Rams finish 8-8-1 and the Niners 8-9 if they lose out: below.
	records[Team::Lar.index()] = Record { w: 8, l: 8, t: 1 };
	records[Team::Sf.index()] = Record { w: 8, l: 8, t: 0 };
	let table = Table::from_parts(
		records,
		vec![Pending {
			week: 1,
			home: Team::Sf,
			away: Team::Kc,
		}],
	);
	assert_eq!(best(&table, Team::Sf, 0).map(|s| s.seed), Some(5));
	assert_eq!(best(&table, Team::Sf, 1).map(|s| s.seed), Some(1));
}

#[test]
fn more_than_the_remaining_games_is_no_path() {
	let table = Table::from_parts(
		[Record::default(); 32],
		vec![Pending {
			week: 1,
			home: Team::Sf,
			away: Team::Sea,
		}],
	);
	assert!(best(&table, Team::Sf, 2).is_none());
}
