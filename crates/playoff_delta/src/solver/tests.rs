//! The solver against brute force: every outcome of a dozen pending games,
//! seeded by the rule itself (PD1: a tie goes to the team asked about).

use super::*;
use crate::table::Pending;

/// Deterministic, dependency-free noise.
struct Lcg(u64);

impl Lcg {
	fn next(&mut self, below: u64) -> u64 {
		self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
		(self.0 >> 33) % below
	}
}

/// The seed `team` gets in final standings `points`, ties going its way.
fn seed_of(points: &[u16; 32], team: Team) -> Option<u8> {
	let p = points[team.index()];
	let above = |t: &Team| points[t.index()] > p;
	let conf: Vec<Team> = Team::of_conference(team.conference()).filter(|t| *t != team).collect();
	let division_tops: Vec<Team> = (0..8)
		.filter(|d| *d != team.division().index())
		.filter_map(|d| conf.iter().filter(|t| t.division().index() == d).max_by_key(|t| points[t.index()]).copied())
		.collect();
	if !team.division_rivals().any(|t| above(&t)) {
		return Some(1 + u8::try_from(division_tops.iter().filter(|t| above(t)).count()).unwrap());
	}
	let own_top = team.division_rivals().max_by_key(|t| points[t.index()]).unwrap();
	let wild = conf.iter().filter(|t| above(t) && **t != own_top && !division_tops.contains(t)).count();
	let seed = 5 + u8::try_from(wild).unwrap();
	(seed <= LAST_SEED).then_some(seed)
}

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

fn random_case(rng: &mut Lcg, games: usize) -> ([Record; 32], Vec<Pending>) {
	let records = [(); 32].map(|()| {
		let w = 6 + u8::try_from(rng.next(5)).unwrap();
		Record { w, l: 14 - w, t: 0 }
	});
	let pending = (0..games)
		.map(|_| {
			// Mostly within one conference, where the seeding is decided.
			let home = Team::ALL[usize::try_from(rng.next(32)).unwrap()];
			let pool: Vec<Team> = if rng.next(4) == 0 {
				Team::ALL.into_iter().filter(|t| *t != home).collect()
			} else {
				Team::of_conference(home.conference()).filter(|t| *t != home).collect()
			};
			let away = pool[usize::try_from(rng.next(pool.len() as u64)).unwrap()];
			Pending { home, away }
		})
		.collect();
	(records, pending)
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
	let table = Table::from_parts(records, vec![Pending { home: Team::Sf, away: Team::Kc }]);
	assert_eq!(best(&table, Team::Sf, 0).map(|s| s.seed), Some(5));
	assert_eq!(best(&table, Team::Sf, 1).map(|s| s.seed), Some(1));
}

#[test]
fn more_than_the_remaining_games_is_no_path() {
	let table = Table::from_parts([Record::default(); 32], vec![Pending { home: Team::Sf, away: Team::Sea }]);
	assert!(best(&table, Team::Sf, 2).is_none());
}
