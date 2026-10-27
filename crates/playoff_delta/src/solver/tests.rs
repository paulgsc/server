//! The solver against brute force: every outcome of a dozen pending games,
//! seeded by the rule itself (PD1: a tie goes to the team asked about), with
//! banked ties in the standings so half-win room is exercised.

use super::*;
use crate::{
	rank::seed as seed_of,
	table::Pending,
	testing::{final_points, random_case, Lcg},
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

/// Clinched against brute force: whenever it says in, every outcome of the
/// pending games seeds the team; and it says so often enough to be tested.
#[test]
fn a_clinch_holds_in_every_outcome() {
	let mut rng = Lcg(0x9F00);
	let mut proven = 0;
	for _ in 0..30 {
		let (records, pending) = random_case(&mut rng, 11);
		let table = Table::from_parts(records, pending.clone());
		for team in Team::ALL.into_iter().filter(|t| clinched(&table, *t)) {
			proven += 1;
			for outcome in 0u32..1 << pending.len() {
				assert!(
					seed_of(&final_points(&records, &pending, outcome), team).is_some(),
					"{}: outcome {outcome:b} leaves it out",
					team.code()
				);
			}
		}
	}
	assert!(proven > 0, "no random league clinched anyone");
}

#[test]
fn a_clinch_by_the_division_or_by_the_wild_card() {
	// Everyone 8-8 with one game left, the Niners 10-6: no one can catch them.
	let mut records = [Record { w: 8, l: 8, t: 0 }; 32];
	records[Team::Sf.index()] = Record { w: 10, l: 6, t: 0 };
	let pending = |a: Team, b: Team| Pending { week: 17, home: a, away: b };
	let table = Table::from_parts(records, vec![pending(Team::Sf, Team::Kc), pending(Team::Lar, Team::Ari)]);
	assert!(clinched(&table, Team::Sf));
	// The Rams 10-6 can pass them, so the division is not proven; with only
	// one other team able to pass, the wild card is.
	records[Team::Lar.index()] = Record { w: 10, l: 6, t: 0 };
	let table = Table::from_parts(records, vec![pending(Team::Sf, Team::Kc), pending(Team::Lar, Team::Ari)]);
	assert!(clinched(&table, Team::Sf));
	// Two teams in each of three other divisions who can pass: one of each
	// pair wins its division, and the other three are wild cards above them.
	let mut games = vec![pending(Team::Sf, Team::Kc), pending(Team::Lar, Team::Ari)];
	for (t, opponent) in [
		(Team::Dal, Team::Buf),
		(Team::Phi, Team::Mia),
		(Team::Det, Team::Ne),
		(Team::Gb, Team::Nyj),
		(Team::Tb, Team::Bal),
		(Team::Atl, Team::Pit),
	] {
		records[t.index()] = Record { w: 10, l: 6, t: 0 };
		games.push(pending(t, opponent));
	}
	let table = Table::from_parts(records, games.clone());
	assert!(!clinched(&table, Team::Sf));
	// One of those pairs short of a game to play: two wild cards above, at most.
	games.retain(|p| p.home != Team::Atl);
	let table = Table::from_parts(records, games);
	assert!(clinched(&table, Team::Sf));
}
