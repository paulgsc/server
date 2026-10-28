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
//! sample and the game's place among the conference's pending games, so two
//! tables with the same pending games (a table and itself with a banked
//! result flipped) are compared on the same sampled ways, and a pending
//! game's two sides are compared on each way with that one game flipped.
//!
//! **Spread** ([`slates`]): a week's games can go 2^k ways, its slates, and
//! each slate leaves a share of its own. The share is a fair game, so those
//! shares average to the share now; their standard deviation is how far the
//! week can move it. Every slate is counted, each against the same rests of
//! the season (the other games), so what the rest decides cancels out and
//! only what the slate decides is left. When the rests are few enough to
//! count, each slate's share is exact. Otherwise they are drawn, and the
//! variance is read from pairs of different rests: two independent rests on
//! one slate both get in with that slate's share squared, which leaves an
//! unbiased estimate.

use crate::{
	league::{Conference, Team},
	rank::seed,
	table::Table,
	ways::Column,
};

/// The share for a table, and per pending game the share given each winner.
#[derive(Debug, Clone, PartialEq)]
pub struct Tally {
	/// 0 to 1.
	pub share: f64,
	/// By index into [`Table::pending`], for the games asked about: the share
	/// had the home side won, then had the away side, both counted over every
	/// way. `None` for a game not asked about, or outside the team's
	/// conference, which cannot change its seed.
	pub given: Vec<Option<(f64, f64)>>,
	/// Every way was counted, not a sample.
	pub exact: bool,
}

/// [`Tally`] for `team` at `table`, over at most `ways` ways, with the
/// per-game shares for the pending games at `games` (indices into
/// [`Table::pending`]).
#[must_use]
pub fn tally(table: &Table, team: Team, ways: u32, games: &[usize]) -> Tally {
	let pending = table.pending();
	let relevant: Vec<usize> = (0..pending.len()).filter(|&i| counts(table, team.conference(), i)).collect();
	// The asked games that can matter, by their slot among the relevant ones.
	let asked: Vec<(usize, usize)> = relevant.iter().enumerate().filter(|(_, i)| games.contains(i)).map(|(slot, &i)| (slot, i)).collect();
	let base = standings(table);
	let (exact, total) = budget(relevant.len(), ways);
	let mut ins = 0u64;
	// Per asked game: the ways in had its home side won, and had its away side.
	let mut sides = vec![(0u64, 0u64); asked.len()];
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
		// The same way with this one game gone the other way: both sides of
		// it are counted over every way, not each over half of them.
		for (side, &(slot, i)) in sides.iter_mut().zip(&asked) {
			let (winner, loser) = if won[slot] {
				(pending[i].home, pending[i].away)
			} else {
				(pending[i].away, pending[i].home)
			};
			let mut other = points;
			other[winner.index()] -= 2;
			other[loser.index()] += 2;
			let flipped = u64::from(seed(&other, team).is_some());
			if won[slot] {
				side.0 += got;
				side.1 += flipped;
			} else {
				side.0 += flipped;
				side.1 += got;
			}
		}
	}
	let ratio = |a: u64| to_f64(a) / to_f64(total);
	let mut given = vec![None; pending.len()];
	for (side, &(_, i)) in sides.iter().zip(&asked) {
		given[i] = Some((ratio(side.0), ratio(side.1)));
	}
	Tally { share: ratio(ins), given, exact }
}

/// A week's slates, each counted against the same rests of the season, for
/// every team asked about (one conference's).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slates {
	/// The slate's games, by index into [`Table::pending`]: bit `j` of a
	/// slate is whether the `j`th one's home side won.
	pub games: Vec<usize>,
	/// The teams counted.
	pub teams: Vec<Team>,
	/// Per slate, then per team: the rests that get the team in, among the
	/// even-numbered rests and among the odd ones.
	ins: Vec<[u32; 2]>,
	/// Per rest, then per team: the slates that get the team in, and of
	/// those the ones with its own game's home side winning (0 for a team
	/// with no game in the slate).
	rests: Vec<(u32, u32)>,
	/// Per team, the bit of its own game in the slate.
	own: Vec<Option<usize>>,
	/// Every rest was counted, not a sample.
	pub exact: bool,
}

/// The most games a slate spans: a week has at most one game per team.
pub const SLATE_GAMES: usize = 16;

/// [`Slates`] for `teams` (one conference's, at least one) at `table`,
/// against at most `rests` rests.
///
/// The slate is the pending games at `games` that can matter to that
/// conference (indices into [`Table::pending`], [`SLATE_GAMES`] at most).
/// `None` past that many, or for no team.
#[must_use]
pub fn slates(table: &Table, teams: &[Team], rests: u32, games: &[usize]) -> Option<Slates> {
	let conference = teams.first()?.conference();
	let pending = table.pending();
	let games: Vec<usize> = games.iter().copied().filter(|&i| counts(table, conference, i)).collect();
	if games.len() > SLATE_GAMES {
		return None;
	}
	let rest_games: Vec<usize> = (0..pending.len()).filter(|&i| counts(table, conference, i) && !games.contains(&i)).collect();
	let (exact, total) = budget(rest_games.len(), rests);
	let base = standings(table);
	let width = teams.len();
	let own: Vec<Option<usize>> = teams.iter().map(|&t| games.iter().position(|&i| pending[i].home == t || pending[i].away == t)).collect();
	let mut ins = vec![[0u32; 2]; width << games.len()];
	let mut out = Vec::with_capacity(usize::try_from(total).unwrap_or(0) * width);
	for k in 0..total {
		// Counted, a rest is its own number; drawn, its bits come from a
		// stream apart from the count's.
		let mut bits = Bits::new(if exact { k } else { k ^ REST_STREAM }, exact);
		let half = usize::from(k & 1 == 1);
		let mut points = base;
		for &i in &rest_games {
			let p = pending[i];
			points[if bits.next() { p.home } else { p.away }.index()] += 2;
		}
		// Every slate in Gray-code order, starting with every away side won:
		// each step turns one game around.
		for &i in &games {
			points[pending[i].away.index()] += 2;
		}
		let mut slate = 0usize;
		let mut got = vec![(0u32, 0u32); width];
		for step in 0..1usize << games.len() {
			if step > 0 {
				let j = usize::try_from(step.trailing_zeros()).unwrap_or(0);
				let p = pending[games[j]];
				slate ^= 1 << j;
				let (winner, loser) = if slate >> j & 1 == 1 { (p.home, p.away) } else { (p.away, p.home) };
				points[winner.index()] += 2;
				points[loser.index()] -= 2;
			}
			let row = &mut ins[slate * width..(slate + 1) * width];
			for (t, &team) in teams.iter().enumerate() {
				if seed(&points, team).is_some() {
					row[t][half] += 1;
					got[t].0 += 1;
					got[t].1 += u32::from(own[t].is_some_and(|j| slate >> j & 1 == 1));
				}
			}
		}
		out.extend(got);
	}
	Some(Slates {
		games,
		teams: teams.to_vec(),
		ins,
		rests: out,
		own,
		exact,
	})
}

/// Turns a rest's number into a stream unrelated to the count's samples.
const REST_STREAM: u64 = 0xA076_1D64_78BD_642F;

impl Slates {
	/// Where `team` sits among the teams counted.
	#[must_use]
	pub fn team(&self, team: Team) -> Option<usize> {
		self.teams.iter().position(|&t| t == team)
	}

	/// The bit of the `t`th team's own game in the slate, if it has one.
	#[must_use]
	pub fn own(&self, t: usize) -> Option<usize> {
		self.own[t]
	}

	/// The rests counted.
	fn total(&self) -> usize {
		self.rests.len() / self.teams.len()
	}

	/// The `t`th team's share per slate, overall and per half of the rests.
	#[must_use]
	pub fn column(&self, t: usize) -> Column {
		let m = self.total();
		// Even-numbered rests are the first half: rest 0, 2, 4...
		let sizes = [m.div_ceil(2), m / 2];
		let width = self.teams.len();
		let of = |n: u32, of: usize| if of == 0 { 0.0 } else { f64::from(n) / to_f64(u64::try_from(of).unwrap_or(u64::MAX)) };
		let cells = || self.ins.iter().skip(t).step_by(width);
		Column {
			games: self.games.len(),
			share: cells().map(|c| of(c[0] + c[1], m)).collect(),
			halves: [cells().map(|c| of(c[0], sizes[0])).collect(), cells().map(|c| of(c[1], sizes[1])).collect()],
			exact: self.exact,
		}
	}

	/// The standard deviation of the `t`th team's share over the slates,
	/// with its own game held to `home_won` when it has one in the slate:
	/// each slate counted once.
	#[must_use]
	pub fn spread(&self, t: usize, home_won: Option<bool>) -> f64 {
		let fixed = home_won.and_then(|h| Some((self.own[t]?, h)));
		let on = |slate: usize| fixed.is_none_or(|(j, h)| (slate >> j & 1 == 1) == h);
		let width = self.teams.len();
		let m = to_f64(u64::try_from(self.total()).unwrap_or(u64::MAX));
		// The variance over the slates of the rests' mean.
		let (mut s, mut sum, mut squares) = (0.0, 0.0, 0.0);
		for slate in (0..1usize << self.games.len()).filter(|&slate| on(slate)) {
			let c = self.ins[slate * width + t];
			let p = f64::from(c[0] + c[1]) / m;
			s += 1.0;
			sum += p;
			squares += p * p;
		}
		let mean = sum / s;
		let between = mean.mul_add(-mean, squares / s);
		let variance = if self.exact || self.total() < 2 {
			between
		} else {
			// Take out each rest's own variance over the slates: what is left
			// is the pairs of different rests, an unbiased estimate.
			let own: f64 = self
				.rests
				.iter()
				.skip(t)
				.step_by(width)
				.map(|&(got, home)| {
					let x = f64::from(match fixed {
						None => got,
						Some((_, true)) => home,
						Some((_, false)) => got - home,
					});
					let p = x / s;
					p * (1.0 - p)
				})
				.sum();
			(m * m).mul_add(between, -own) / (m * (m - 1.0))
		};
		// A sampled estimate can dip just below zero.
		variance.max(0.0).sqrt()
	}
}

/// Can pending game `i` change the seed of a team in `conference`: does it
/// play in it.
fn counts(table: &Table, conference: Conference, i: usize) -> bool {
	let p = table.pending()[i];
	p.home.conference() == conference || p.away.conference() == conference
}

/// Every team's points at `table`, by [`Team::index`].
fn standings(table: &Table) -> [u16; 32] {
	let mut points = [0u16; 32];
	for t in Team::ALL {
		points[t.index()] = table.points(t);
	}
	points
}

/// Whether `games` games' every outcome fits in `cap` ways (then all are
/// counted), and how many ways that is.
fn budget(games: usize, cap: u32) -> (bool, u64) {
	let exact = u32::try_from(games).is_ok_and(|n| n < 32 && 1u64 << n <= u64::from(cap.max(1)));
	(exact, if exact { 1u64 << games } else { u64::from(cap.max(1)) })
}

/// [`Tally::share`] alone, without the per-game shares.
#[must_use]
pub fn share(table: &Table, team: Team, ways: u32) -> f64 {
	tally(table, team, ways, &[]).share
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
