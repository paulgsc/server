//! Test support shared by the modules' tests: noise and random leagues.

use crate::{
	league::Team,
	table::{Pending, Record},
};

/// Deterministic, dependency-free noise.
pub(crate) struct Lcg(pub(crate) u64);

impl Lcg {
	pub(crate) fn next(&mut self, below: u64) -> u64 {
		self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
		(self.0 >> 33) % below
	}
}

/// Banked records for every team (some with a tie, so half-win room is
/// exercised) and `games` pending games, mostly within one conference.
pub(crate) fn random_case(rng: &mut Lcg, games: usize) -> ([Record; 32], Vec<Pending>) {
	let records = [(); 32].map(|()| {
		let w = 6 + u8::try_from(rng.next(5)).unwrap();
		let t = u8::from(rng.next(4) == 0);
		Record { w, l: 14 - w - t, t }
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
			Pending { week: 1, home, away }
		})
		.collect();
	(records, pending)
}

/// Final points for one outcome of `pending`: bit `i` of `outcome` clear when
/// game `i`'s home side won.
pub(crate) fn final_points(records: &[Record; 32], pending: &[Pending], outcome: u32) -> [u16; 32] {
	let mut points = records.map(Record::points);
	for (i, p) in pending.iter().enumerate() {
		points[(if outcome & (1 << i) == 0 { p.home } else { p.away }).index()] += 2;
	}
	points
}
