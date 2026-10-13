//! An invented season: a round-robin schedule (the circle method, so nobody
//! meets twice) with scores from a fixed-seed generator. Consistent, not real.

use playoff_delta::{
	league::Team,
	season::{Game, Score, Season, SeasonError},
};

pub(crate) struct Lcg(pub(crate) u64);

impl Lcg {
	pub(crate) const fn next(&mut self, below: u64) -> u64 {
		self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
		(self.0 >> 33) % below
	}
}

/// 17 weeks, every team every week, final through `through`.
pub(crate) fn invented(seed: u64, through: u8) -> Result<Season, SeasonError> {
	Season::new(invented_games(seed, through))
}

/// The same games, all final, over 18 weeks: half of week 9 moves to a new
/// week 10 (later weeks shift one on), so every team has one bye.
pub(crate) fn with_byes(seed: u64) -> Result<Season, SeasonError> {
	let mut games = invented_games(seed, 17);
	let mut moved = 0;
	for g in &mut games {
		if g.week > 9 {
			g.week += 1;
		} else if g.week == 9 {
			moved += 1;
			if moved > 8 {
				g.week = 10;
			}
		}
	}
	Season::new(games)
}

fn invented_games(seed: u64, through: u8) -> Vec<Game> {
	let mut noise = Lcg(seed);
	// Team strength, so standings spread the way a real season's do.
	let strength: Vec<u64> = (0..32).map(|_| noise.next(10)).collect();
	let mut ring: Vec<usize> = (0..32).collect();
	let mut games = Vec::new();
	for week in 1..=17u8 {
		for i in 0..16 {
			let (a, b) = (ring[i], ring[31 - i]);
			let (home, away) = if week % 2 == 0 { (a, b) } else { (b, a) };
			let score = (week <= through).then(|| {
				let h = 10 + strength[home] + noise.next(21);
				let mut v = 10 + strength[away] + noise.next(21);
				if v == h {
					v += 3;
				}
				Score {
					home: u16::try_from(h).unwrap_or_default(),
					away: u16::try_from(v).unwrap_or_default(),
				}
			});
			games.push(Game {
				week,
				home: Team::ALL[home],
				away: Team::ALL[away],
				score,
				collapse: None,
			});
		}
		if let Some(last) = ring.pop() {
			ring.insert(1, last);
		}
	}
	games
}
