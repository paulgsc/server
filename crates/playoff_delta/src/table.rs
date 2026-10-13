//! The league at one instant: what every team has banked, and which games are
//! still to play.
//!
//! Standing is counted in points, two for a win and one for a tie, so a tie
//! is exact half-win arithmetic with no floats.

use crate::{
	league::Team,
	season::{Game, Outcome, Season},
};

/// A game still to play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pending {
	pub home: Team,
	pub away: Team,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Record {
	pub w: u8,
	pub l: u8,
	pub t: u8,
}

impl Record {
	#[must_use]
	pub const fn points(self) -> u16 {
		self.w as u16 * 2 + self.t as u16
	}
}

#[derive(Debug, Clone)]
pub struct Table {
	records: [Record; 32],
	pending: Vec<Pending>,
	through: u8,
}

impl Table {
	/// Every game through `week` is banked; the rest is pending.
	#[must_use]
	pub fn after(season: &Season, week: u8) -> Self {
		let mut table = Self {
			records: [Record::default(); 32],
			pending: Vec::new(),
			through: week,
		};
		for g in season.games() {
			match g.outcome().filter(|_| g.week <= week) {
				Some(outcome) => table.bank(g, outcome),
				None => table.pending.push(Pending { home: g.home, away: g.away }),
			}
		}
		table
	}

	/// The same table with banked `game` decided as `outcome` instead.
	#[must_use]
	pub fn decided(&self, game: &Game, outcome: Outcome) -> Self {
		let mut table = self.clone();
		if let Some(actual) = game.outcome() {
			table.unbank(game, actual);
			table.bank(game, outcome);
		}
		table
	}

	fn unbank(&mut self, g: &Game, outcome: Outcome) {
		match outcome {
			Outcome::Won(winner) => {
				self.records[winner.index()].w -= 1;
				self.records[g.opponent(winner).index()].l -= 1;
			}
			Outcome::Tied => {
				self.records[g.home.index()].t -= 1;
				self.records[g.away.index()].t -= 1;
			}
		}
	}

	fn bank(&mut self, g: &Game, outcome: Outcome) {
		match outcome {
			Outcome::Won(winner) => {
				self.records[winner.index()].w += 1;
				self.records[g.opponent(winner).index()].l += 1;
			}
			Outcome::Tied => {
				self.records[g.home.index()].t += 1;
				self.records[g.away.index()].t += 1;
			}
		}
	}

	#[must_use]
	pub const fn record(&self, team: Team) -> Record {
		self.records[team.index()]
	}

	#[must_use]
	pub const fn points(&self, team: Team) -> u16 {
		self.records[team.index()].points()
	}

	/// The last week banked. Two tables of one season with the same `through`
	/// have the same pending games, whatever results were decided otherwise.
	#[must_use]
	pub const fn through(&self) -> u8 {
		self.through
	}

	#[must_use]
	pub fn pending(&self) -> &[Pending] {
		&self.pending
	}

	/// Games `team` has left.
	#[must_use]
	pub fn remaining(&self, team: Team) -> u8 {
		u8::try_from(self.pending.iter().filter(|p| p.home == team || p.away == team).count()).unwrap_or(u8::MAX)
	}
}

#[cfg(test)]
impl Table {
	/// A table from its parts, for tests that need standings no real
	/// schedule would produce.
	pub(crate) const fn from_parts(records: [Record; 32], pending: Vec<Pending>) -> Self {
		Self { records, pending, through: 0 }
	}
}
