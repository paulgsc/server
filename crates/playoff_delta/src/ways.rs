//! A week's ways, read for one team: the best way the week could have gone,
//! what each game was worth to it, how its results took the week from there,
//! and where the way it went ranks among the rest.
//!
//! The input is one team's column of [`crate::count::Slates`]: per slate (a
//! way the week's results can go, bit `j` set when the `j`th game's home side
//! won), the share it leaves, counted against shared rests of the season,
//! and the same read off each half of those rests. Everything here is a max,
//! a mean or a count over those numbers, so the derivations the stage shows
//! from them add up exactly.
//!
//! - **Best**: the highest share any way reaches. With rests sampled, many
//!   ways land within the sample's error of it, so which single way is best
//!   is noise. What is stable is each game's **price**: the best share with
//!   the game won by the side the best way wants, minus the best with it won
//!   by the other side.
//! - **Rank**: the share of the ways that leave the team worse than the way
//!   the week went, ties counted half.
//! - **Road**: the best share still reachable before any result, then after
//!   each, in kickoff order. It never rises; its first point is the best and
//!   its last the actual share, so its drops add up to the gap between them.

/// The bins a strip's histogram has.
pub const BINS: usize = 20;

/// One team's shares over a week's slates.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
	/// The week's games.
	pub games: usize,
	/// Per slate, the share it leaves.
	pub share: Vec<f64>,
	/// The same, read off each half of the rests.
	pub halves: [Vec<f64>; 2],
	/// Every rest was counted: a price has no error.
	pub exact: bool,
}

/// A game's place in the best way the slates `on` keeps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Piece {
	/// The side the best way wants won.
	pub want_home: bool,
	/// The best with the game won by that side, minus the best with it won
	/// by the other; 0 when `on` keeps only one side.
	pub price: f64,
	/// Half the gap between the price read off each half of the rests.
	pub error: f64,
}

/// The best way among the slates `on` keeps, and every game's piece in it.
#[derive(Debug, Clone, PartialEq)]
pub struct Best {
	pub best: f64,
	pub pieces: Vec<Piece>,
}

/// Where the actual way leaves the team, read one result at a time.
#[derive(Debug, Clone, PartialEq)]
pub struct Road {
	/// The best share still reachable: before any result, then after each.
	pub best: Vec<f64>,
	/// The mean share of the ways still possible, at the same points.
	pub mean: Vec<f64>,
	/// At the same points, the ways still possible per bin of [`BINS`] over
	/// `[worst, best]`.
	pub strip: Vec<Vec<u32>>,
}

impl Column {
	const fn slates(&self) -> usize {
		self.share.len()
	}

	/// The highest share among the slates `on` keeps (0 when it keeps none).
	fn max(values: &[f64], slates: impl Iterator<Item = usize>) -> f64 {
		slates.map(|s| values[s]).fold(f64::NEG_INFINITY, f64::max).max(0.0)
	}

	/// The best way among the slates `on` keeps, and each game's piece.
	#[must_use]
	pub fn best(&self, on: impl Fn(usize) -> bool) -> Best {
		let kept = || (0..self.slates()).filter(|&s| on(s));
		let best = Self::max(&self.share, kept());
		let pieces = (0..self.games)
			.map(|j| {
				let side = |values: &[f64], home: bool| {
					let mut it = kept().filter(|s| (s >> j & 1 == 1) == home).peekable();
					it.peek().is_some().then(|| Self::max(values, it))
				};
				match (side(&self.share, true), side(&self.share, false)) {
					(Some(home), Some(away)) => {
						let want_home = home >= away;
						let price_of = |values: &[f64]| match (side(values, want_home), side(values, !want_home)) {
							(Some(want), Some(other)) => want - other,
							_ => 0.0,
						};
						let error = if self.exact {
							0.0
						} else {
							(price_of(&self.halves[0]) - price_of(&self.halves[1])).abs() / 2.0
						};
						Piece {
							want_home,
							price: (home - away).abs(),
							error,
						}
					}
					// `on` holds this game: its only side is the one wanted.
					(home, _) => Piece {
						want_home: home.is_some(),
						price: 0.0,
						error: 0.0,
					},
				}
			})
			.collect();
		Best { best, pieces }
	}

	/// The lowest share any slate leaves.
	#[must_use]
	pub fn worst(&self) -> f64 {
		self.share.iter().copied().fold(f64::INFINITY, f64::min)
	}

	/// Of the slates `on` keeps, the share that leave the team worse than
	/// slate `actual`, ties counted half.
	#[must_use]
	pub fn rank(&self, actual: usize, on: impl Fn(usize) -> bool) -> f64 {
		let at = self.share[actual];
		let (mut kept, mut below) = (0u32, 0u32);
		for s in (0..self.slates()).filter(|&s| on(s)) {
			kept += 2;
			below += match self.share[s].total_cmp(&at) {
				std::cmp::Ordering::Less => 2,
				std::cmp::Ordering::Equal => 1,
				std::cmp::Ordering::Greater => 0,
			};
		}
		if kept == 0 {
			0.0
		} else {
			f64::from(below) / f64::from(kept)
		}
	}

	/// The road from the best way to slate `actual`, revealing the games in
	/// `order` (bits of the slate, in kickoff order).
	#[must_use]
	pub fn road(&self, actual: usize, order: &[usize]) -> Road {
		let (worst, best) = (self.worst(), Self::max(&self.share, 0..self.slates()));
		let bin = |v: f64| {
			if best > worst {
				// Truncation is the binning: a share at `best` lands in the last bin.
				#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
				let b = ((v - worst) / (best - worst) * to_f64(BINS)) as usize;
				b.min(BINS - 1)
			} else {
				BINS - 1
			}
		};
		let mut road = Road {
			best: Vec::with_capacity(order.len() + 1),
			mean: Vec::with_capacity(order.len() + 1),
			strip: Vec::with_capacity(order.len() + 1),
		};
		let mut mask = 0usize;
		for p in 0..=order.len() {
			if p > 0 {
				mask |= 1 << order[p - 1];
			}
			let (mut top, mut sum, mut n, mut counts) = (f64::NEG_INFINITY, 0.0, 0u32, vec![0u32; BINS]);
			for s in (0..self.slates()).filter(|s| s & mask == actual & mask) {
				let v = self.share[s];
				top = top.max(v);
				sum += v;
				n += 1;
				counts[bin(v)] += 1;
			}
			road.best.push(top);
			road.mean.push(sum / f64::from(n));
			road.strip.push(counts);
		}
		road
	}
}

/// Small counts as floats, exactly.
#[allow(clippy::cast_precision_loss)]
const fn to_f64(n: usize) -> f64 {
	n as f64
}

#[cfg(test)]
mod tests;
