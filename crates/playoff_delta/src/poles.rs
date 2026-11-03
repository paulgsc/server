//! The poles: how few and how many wins a group of rivals can finish with,
//! and how low a team can still fall, each read exactly off the games left.
//!
//! Nothing here weighs an outcome. A question is asked over every way the
//! rest of the season can go, and answered with the extreme and its proof:
//!
//! - **The floor** of a group, the best case for the team the story follows:
//!   that team wins out, every member loses each game against a team outside
//!   the group, and each game between two members still hands one of them
//!   the win. The floor is the fewest points the group's best member can then
//!   be held to. A game between two members is the forcing: whichever way it
//!   goes, a member gets it.
//! - **Its bloc**: members whose games among themselves force the floor on
//!   their own. They have `banked` points and `games` left against each
//!   other, and however those go, one of them reaches the floor (Hakimi's
//!   orientation theorem, 1965: the same max-flow and min cut as
//!   [`crate::solver`]).
//! - **The ceiling** of a group, the worst case: the most points a member can
//!   reach, by winning every game it has left. One team winning out conflicts
//!   with nobody, so no forcing applies.
//! - **The worst seed**: the team loses out, and as many conference teams as
//!   can all finish above it at once do. A set of teams that cannot all
//!   finish above makes every larger set impossible too, so the search grows
//!   only sets that can, one flow each.
//!
//! [`Field`] is league-free: entities by index, their banked points (a win 2,
//! a tie 1), and the games left as pairs of entities. A game left hands out
//! two points to one side, never a tie (the solver's model). The NFL is
//! adapted to it in [`crate::evaluate`].

use crate::{flow::Network, rank};

/// Banked points per entity, and the games still to play.
#[derive(Debug, Clone, Copy)]
pub struct Field<'a> {
	pub points: &'a [u16],
	pub games: &'a [(usize, usize)],
}

/// A group's floor, with the bloc that proves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Floor {
	/// The fewest points the group's best member can be held to.
	pub points: u16,
	/// Members whose games among themselves force `points` on their own, in
	/// the group's order. None can be left out without lowering their floor.
	pub bloc: Vec<usize>,
	/// The bloc's banked points.
	pub banked: u16,
	/// Games the bloc's members have left against each other.
	pub games: u16,
}

/// A group's ceiling: the most points a member can reach, and the first
/// member, in the group's order, that reaches it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ceiling {
	pub points: u16,
	pub by: usize,
}

/// A cap on each member's final points, or the members whose games among
/// themselves overflow it.
enum Placed {
	Yes,
	No(Vec<usize>),
}

impl Field<'_> {
	/// Games `e` has left.
	fn left(&self, e: usize) -> u16 {
		u16::try_from(self.games.iter().filter(|&&(a, b)| a == e || b == e).count()).unwrap_or(u16::MAX)
	}

	/// Games `a` and `b` have left against each other.
	fn between(&self, a: usize, b: usize) -> u16 {
		u16::try_from(self.games.iter().filter(|&&(x, y)| (x, y) == (a, b) || (x, y) == (b, a)).count()).unwrap_or(u16::MAX)
	}

	/// The games with both sides in `group`, by index into `self.games`.
	fn inside(&self, group: &[usize]) -> Vec<usize> {
		(0..self.games.len())
			.filter(|&g| group.contains(&self.games[g].0) && group.contains(&self.games[g].1))
			.collect()
	}

	/// Can the games inside `group` be decided with every member finishing
	/// at or under its cap?
	fn place(&self, group: &[usize], cap: impl Fn(usize) -> u16) -> Placed {
		let inside = self.inside(group);
		let (source, sink) = (0, 1);
		let node = |i: usize| 2 + i;
		let mut net = Network::new(2 + group.len() + inside.len());
		for (i, &m) in group.iter().enumerate() {
			let Some(room) = cap(m).checked_sub(self.points[m]) else {
				return Placed::No(vec![m]);
			};
			// A game left hands out a whole win, so odd room left by a banked
			// tie is half a win nobody can take.
			net.add(node(i), sink, u32::from(room) & !1);
		}
		let at = |e: usize| group.iter().position(|&m| m == e).unwrap_or(0);
		for (k, &g) in inside.iter().enumerate() {
			let (a, b) = self.games[g];
			let game = 2 + group.len() + k;
			net.add(source, game, 2);
			net.add(game, node(at(a)), 2);
			net.add(game, node(at(b)), 2);
		}
		let need = u32::try_from(inside.len()).unwrap_or(u32::MAX) * 2;
		if net.max_flow(source, sink) == need {
			return Placed::Yes;
		}
		// The members on the source side of a minimum cut: every game they
		// can still reach lies between two of them, and those games hand out
		// more than their caps hold.
		let side = net.reachable(source);
		Placed::No(group.iter().enumerate().filter(|(i, _)| side[node(*i)]).map(|(_, &m)| m).collect())
	}

	/// The fewest points `group`'s best member can finish with, a member
	/// listed in `held` finishing at or under its points there instead;
	/// `None` for an empty group or one `held` cannot keep.
	#[must_use]
	pub fn floor_held(&self, group: &[usize], held: &[(usize, u16)]) -> Option<u16> {
		let cap = |c: u16| move |m: usize| held.iter().find(|(h, _)| *h == m).map_or(c, |(_, p)| c.min(*p));
		let mut lo = group.iter().map(|&m| self.points[m]).max()?;
		let mut hi = lo + 2 * u16::try_from(self.inside(group).len()).unwrap_or(u16::MAX);
		if matches!(self.place(group, cap(hi)), Placed::No(_)) {
			return None;
		}
		// Feasible caps only grow feasible, so the least is a binary search;
		// at the least one, the best member finishes exactly on it.
		while lo < hi {
			let mid = lo + (hi - lo) / 2;
			match self.place(group, cap(mid)) {
				Placed::Yes => hi = mid,
				Placed::No(_) => lo = mid + 1,
			}
		}
		Some(lo)
	}

	/// [`Self::floor_held`] with nobody held.
	#[must_use]
	pub fn floor_points(&self, group: &[usize]) -> Option<u16> {
		self.floor_held(group, &[])
	}

	/// `group`'s floor and its bloc; `None` for an empty group.
	#[must_use]
	pub fn floor(&self, group: &[usize]) -> Option<Floor> {
		let points = self.floor_points(group)?;
		// One under the floor, the cut names members whose own games force it.
		let mut bloc = match points.checked_sub(1).map(|below| self.place(group, |_| below)) {
			Some(Placed::No(cut)) => cut,
			// No member has any points and no game is left inside.
			_ => group.to_vec(),
		};
		// The fewest that still force it: leave out each member in turn while
		// the rest keep the floor on their own.
		let mut i = 0;
		while i < bloc.len() && bloc.len() > 1 {
			let mut rest = bloc.clone();
			rest.remove(i);
			if self.floor_points(&rest) == Some(points) {
				bloc = rest;
			} else {
				i += 1;
			}
		}
		let banked = bloc.iter().map(|&m| self.points[m]).sum();
		let games = u16::try_from(self.inside(&bloc).len()).unwrap_or(u16::MAX);
		Some(Floor { points, bloc, banked, games })
	}

	/// The most points a member of `group` can reach. With `winning_out`, the
	/// games each plays against `subject` are its losses.
	#[must_use]
	pub fn ceiling(&self, group: &[usize], subject: usize, winning_out: bool) -> Option<Ceiling> {
		let reach = |m: usize| {
			let against = if winning_out { self.between(m, subject) } else { 0 };
			self.points[m] + 2 * (self.left(m) - against)
		};
		let points = group.iter().map(|&m| reach(m)).max()?;
		let by = group.iter().copied().find(|&m| reach(m) == points)?;
		Some(Ceiling { points, by })
	}

	/// The worst seed `subject` can finish with, whatever happens: it loses
	/// every game it has left, and the most conference teams that can all
	/// finish strictly above it at once do (PD1: a tie goes to it). `None`:
	/// some way the season can go leaves it out.
	///
	/// `conference` lists the subject's conference, itself included;
	/// `division` gives each entity's division (any numbering, the same
	/// within the conference).
	#[must_use]
	pub fn worst_seed(&self, conference: &[usize], division: impl Fn(usize) -> usize, subject: usize) -> Option<u8> {
		let others: Vec<usize> = conference.iter().copied().filter(|&e| e != subject).collect();
		let divisions: Vec<usize> = {
			let mut d: Vec<usize> = conference.iter().map(|&e| division(e)).collect();
			d.sort_unstable();
			d.dedup();
			d
		};
		let slot = |e: usize| divisions.iter().position(|&d| d == division(e)).unwrap_or(0);
		let mut search = Worst {
			field: self,
			others: &others,
			floor: self.points[subject],
			own: slot(subject),
			slot: &slot,
			divisions: divisions.len(),
			found: Some(1),
		};
		search.grow(&mut Vec::new(), 0);
		search.found
	}

	/// Can every team of `set` finish strictly above `floor` at once? Each
	/// takes its games against teams outside the set; games inside the set
	/// must cover what each still needs.
	fn all_above(&self, set: &[usize], floor: u16) -> bool {
		let inside = self.inside(set);
		let (source, sink) = (0, 1);
		let mut net = Network::new(2 + set.len() + inside.len());
		let mut need_total = 0u32;
		for (i, &m) in set.iter().enumerate() {
			// Wins to pass `floor`: strictly more points.
			let need = if self.points[m] > floor { 0 } else { (floor - self.points[m]) / 2 + 1 };
			let ours = u16::try_from(inside.iter().filter(|&&g| self.games[g].0 == m || self.games[g].1 == m).count()).unwrap_or(u16::MAX);
			let need = u32::from(need.saturating_sub(self.left(m) - ours));
			need_total += need;
			net.add(2 + i, sink, need);
		}
		let at = |e: usize| set.iter().position(|&m| m == e).unwrap_or(0);
		for (k, &g) in inside.iter().enumerate() {
			let (a, b) = self.games[g];
			let game = 2 + set.len() + k;
			net.add(source, game, 1);
			net.add(game, 2 + at(a), 1);
			net.add(game, 2 + at(b), 1);
		}
		net.max_flow(source, sink) == need_total
	}
}

/// The worst-seed search: every set of conference teams that can all finish
/// above the subject at once, grown one team at a time.
struct Worst<'a, F> {
	field: &'a Field<'a>,
	others: &'a [usize],
	/// The subject's points, losing out.
	floor: u16,
	own: usize,
	slot: &'a F,
	divisions: usize,
	/// The worst seed found so far; `None` once out.
	found: Option<u8>,
}

impl<F: Fn(usize) -> usize> Worst<'_, F> {
	/// Grows `set` by the teams from `from` on. `false` once the subject is
	/// out, which nothing can make worse.
	fn grow(&mut self, set: &mut Vec<usize>, from: usize) -> bool {
		let mut above = vec![0u8; self.divisions];
		for &m in set.iter() {
			above[(self.slot)(m)] += 1;
		}
		match rank::from_above(&above, self.own) {
			None => {
				self.found = None;
				return false;
			}
			Some(seed) => self.found = self.found.map(|w| w.max(seed)),
		}
		for j in from..self.others.len() {
			set.push(self.others[j]);
			let go_on = !self.field.all_above(set, self.floor) || self.grow(set, j + 1);
			set.pop();
			if !go_on {
				return false;
			}
		}
		true
	}
}

#[cfg(test)]
mod tests;
