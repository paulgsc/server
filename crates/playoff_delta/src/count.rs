//! How often: of the ways the rest of the season can go, the share in which
//! a team gets in.
//!
//! The solver ([`crate::solver::best`]) asks whether any way in exists, and
//! a result can only remove ways, so that answer never improves. Counting
//! asks how many: every way the remaining games can go counts once (no team
//! strengths, so this is counting, not forecasting), each is seeded by
//! [`crate::rank::seed`] under the same standings model (PD1), and a result's
//! effect is the share as it happened minus the share had it gone the other
//! way. Over a week the expected change is zero, so its sign is the luck.
//!
//! Only games involving the team's conference can change its seed, so only
//! they are counted. When there are few enough of them every way is counted
//! (exact); otherwise `ways` of them are drawn from a fixed-seed generator
//! (PD2: the same table gives the same share). The draw depends only on the
//! sample and the game's place among the pending games, so two tables with
//! the same pending games (a table and itself with a banked result flipped)
//! are compared on the same sampled ways.

use crate::{league::Team, rank::seed, table::Table};

/// The share for a table, and per pending game the share given each winner.
#[derive(Debug, Clone, PartialEq)]
pub struct Tally {
	/// 0 to 1.
	pub share: f64,
	/// By index into [`Table::pending`]: the share given the home side wins,
	/// then given the away side wins; `None` for a game outside the team's
	/// conference, which cannot change its seed, and for every game when
	/// only the share was asked for.
	pub given: Vec<Option<(f64, f64)>>,
	/// Every way was counted, not a sample.
	pub exact: bool,
}

/// [`Tally`] for `team` at `table`, over at most `ways` ways.
#[must_use]
pub fn tally(table: &Table, team: Team, ways: u32) -> Tally {
	count(table, team, ways, true)
}

/// [`Tally::share`] alone, without the per-game shares.
#[must_use]
pub fn share(table: &Table, team: Team, ways: u32) -> f64 {
	count(table, team, ways, false).share
}

fn count(table: &Table, team: Team, ways: u32, per_game: bool) -> Tally {
	let conference = team.conference();
	let pending = table.pending();
	let relevant: Vec<usize> = (0..pending.len())
		.filter(|&i| pending[i].home.conference() == conference || pending[i].away.conference() == conference)
		.collect();
	let mut base = [0u16; 32];
	for t in Team::ALL {
		base[t.index()] = table.points(t);
	}
	let exact = u32::try_from(relevant.len()).is_ok_and(|n| n < 32 && 1u64 << n <= u64::from(ways.max(1)));
	let total: u64 = if exact { 1u64 << relevant.len() } else { u64::from(ways.max(1)) };
	let mut ins = 0u64;
	// Per relevant game: ways the home side won, and how many of those got in.
	let mut home = vec![(0u64, 0u64); relevant.len()];
	let mut won = vec![false; relevant.len()];
	for k in 0..total {
		let mut bits = Bits::new(k, exact);
		let mut points = base;
		for (slot, &i) in relevant.iter().enumerate() {
			let h = bits.next();
			won[slot] = h;
			let winner = if h { pending[i].home } else { pending[i].away };
			points[winner.index()] += 2;
		}
		let got = u64::from(seed(&points, team).is_some());
		ins += got;
		if !per_game {
			continue;
		}
		for (h, count) in won.iter().zip(home.iter_mut()) {
			if *h {
				count.0 += 1;
				count.1 += got;
			}
		}
	}
	let ratio = |a: u64, b: u64| if b == 0 { 0.0 } else { to_f64(a) / to_f64(b) };
	let mut given = vec![None; pending.len()];
	for (slot, &i) in relevant.iter().enumerate().filter(|_| per_game) {
		let (n, got) = home[slot];
		given[i] = Some((ratio(got, n), ratio(ins - got, total - n)));
	}
	Tally {
		share: ratio(ins, total),
		given,
		exact,
	}
}

/// Counts as floats: every count here is far below 2^53, so this is exact.
#[allow(clippy::cast_precision_loss)]
const fn to_f64(n: u64) -> f64 {
	n as f64
}

/// One way's outcomes, a bit per relevant game: the way's own number when
/// every way is counted, a fixed-seed generator's bits otherwise.
struct Bits {
	state: u64,
	exact: bool,
	left: u32,
	word: u64,
}

impl Bits {
	const fn new(k: u64, exact: bool) -> Self {
		Self {
			// splitmix64 of the sample's number: unrelated streams per sample.
			state: if exact { k } else { mix(k.wrapping_add(0x9E37_79B9_7F4A_7C15)) },
			exact,
			left: 64,
			word: 0,
		}
	}

	const fn next(&mut self) -> bool {
		if self.exact {
			let b = self.state & 1 == 1;
			self.state >>= 1;
			return b;
		}
		if self.left == 64 {
			self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
			self.word = mix(self.state);
			self.left = 0;
		}
		let b = (self.word >> self.left) & 1 == 1;
		self.left += 1;
		b
	}
}

const fn mix(mut z: u64) -> u64 {
	z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
	z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
	z ^ (z >> 31)
}

#[cfg(test)]
mod tests;
