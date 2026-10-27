//! How high can a team still finish? The one hard question, answered exactly.
//!
//! [`best`] asks: if `team` wins exactly `extra` of its remaining games and
//! every other remaining game goes whichever way suits it, what is the best
//! playoff seed it can reach? The story's paths are this question asked of
//! each count of more wins. [`clinched`] asks the opposite, worst case.
//!
//! ## The model (invariant PD1)
//!
//! A tie in the final standings goes to the team being asked about. That is
//! the "still alive" reading: a path is open while the team can finish level
//! with or ahead of everyone it must beat, and the NFL's tiebreakers (head to
//! head, division record, common games, strength of victory...) are not
//! applied. The answer is therefore the best seed the standings allow, which
//! is never worse than what the tiebreakers would give. Banked ties count half
//! a win; remaining games are decided as wins and losses, so a team's room
//! below `F` is rounded down to whole wins.
//!
//! ## How
//!
//! Only the asked team's conference matters for its seed. Caps follow from
//! one number, its final points `F`: a team capped at `F` finishes at or under
//! the asked team. Whether every remaining game can be placed without any
//! capped team passing its cap is a max-flow (the classic elimination network,
//! [`crate::flow`]). Which teams are *allowed* above `F` is the search:
//!
//! - **Division winner (seeds 1-4).** No division rival above `F`, and the
//!   seed is one plus the number of other divisions allowed a team above.
//!   Eight subsets, tried smallest first.
//! - **Wild card (seeds 5-7).** Each division may put its winner above `F`
//!   for free; each further team above costs one seed. A branch-and-bound
//!   over the set allowed above: when the flow fails, its minimum cut names
//!   a set of capped teams whose games among themselves exceed their caps,
//!   and every feasible superset must free one of them, so branching on
//!   exactly those is complete.

use crate::{
	flow::Network,
	league::Team,
	table::{Record, Table},
};
use std::{
	cell::RefCell,
	collections::{HashMap, HashSet},
};

/// The worst seed that still makes the playoffs.
pub const LAST_SEED: u8 = 7;

/// One way in: the seed, and how every team's remaining games went to get there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scenario {
	pub seed: u8,
	/// Each team's record over its remaining games, indexed by [`Team::index`].
	pub rest: [Record; 32],
}

/// The best seed `team` can reach winning exactly `extra` more games, with a
/// season that gets it there; `None` when no way in exists.
#[must_use]
pub fn best(table: &Table, team: Team, extra: u8) -> Option<Scenario> {
	Ask::new(table, team, extra)?.solve()
}

/// Is `team` in whatever happens: does every way the rest of the season can
/// go get it in? `true` only when proven; `false` is "not proven".
///
/// Its worst case is losing every game it has left: turning one of its wins
/// into a loss lowers it and raises an opponent, so no team that was below it
/// gets above. Losing out, a team can finish above it (PD1: strictly more
/// points) when its points plus every game it has left beat the asked team's.
/// [`crate::rank::seed`] then reads the proof: with no division mate able to
/// finish above, it wins its division; otherwise its wild card is 5 plus,
/// per division, the teams above it but one, so it is in while at most two
/// such teams can be above. Both counts ignore the games those teams play
/// each other, so they can miss a clinch, never claim a false one.
#[must_use]
pub fn clinched(table: &Table, team: Team) -> bool {
	let floor = table.points(team);
	let mut above = [0u8; 8];
	for t in Team::of_conference(team.conference()).filter(|t| *t != team) {
		if table.points(t) + u16::from(table.remaining(t)) * 2 > floor {
			above[t.division().index()] += 1;
		}
	}
	let past_the_division_winners: u8 = above.iter().map(|a| a.saturating_sub(1)).sum();
	above[team.division().index()] == 0 || past_the_division_winners + 5 <= LAST_SEED
}

/// What [`best`] depends on: the pending games (fixed by `through` within
/// one season), the asked team's conference's points, the team and `extra`.
type Key = (u8, [u16; 16], Team, u8);

/// [`best`], remembered, for the tables of one season.
///
/// The evaluator asks the same question many times: a flipped result in the
/// other conference leaves this conference's points, and so the answer,
/// unchanged.
#[derive(Debug, Default)]
pub struct Memo {
	seen: RefCell<HashMap<Key, Option<Scenario>>>,
}

impl Memo {
	/// [`best`] for `table`, which must come from the season this memo serves.
	#[must_use]
	pub fn best(&self, table: &Table, team: Team, extra: u8) -> Option<Scenario> {
		let mut points = [0u16; 16];
		for (slot, t) in points.iter_mut().zip(Team::of_conference(team.conference())) {
			*slot = table.points(t);
		}
		let key = (table.through(), points, team, extra);
		if let Some(hit) = self.seen.borrow().get(&key) {
			return hit.clone();
		}
		let answer = best(table, team, extra);
		self.seen.borrow_mut().insert(key, answer.clone());
		answer
	}
}

/// Bitmask over a conference's sixteen teams, by position in [`Ask::conf`].
type Mask = u16;

struct Ask<'a> {
	table: &'a Table,
	team: Team,
	extra: u8,
	/// The asked team's final points.
	target: u16,
	conf: Vec<Team>,
	me: usize,
	/// Conference positions of each of its four divisions' teams.
	divisions: [Mask; 4],
	own_division: usize,
	/// Teams already past `target`: above it whatever happens.
	forced: Mask,
}

enum Placed {
	/// Every game placed: per pending game the flow decided, its winner.
	Yes(Vec<(usize, Team)>),
	/// The capped teams (the asked one included, when there) on the source
	/// side of a minimum cut: their games among themselves overflow their caps.
	No(Mask),
}

impl<'a> Ask<'a> {
	fn new(table: &'a Table, team: Team, extra: u8) -> Option<Self> {
		if extra > table.remaining(team) {
			return None;
		}
		let conf: Vec<Team> = Team::of_conference(team.conference()).collect();
		let me = conf.iter().position(|t| *t == team)?;
		let target = table.points(team) + u16::from(extra) * 2;
		let mut divisions: [Mask; 4] = [0; 4];
		for (i, t) in conf.iter().enumerate() {
			divisions[t.division().index() % 4] |= 1 << i;
		}
		let forced = conf
			.iter()
			.enumerate()
			.filter(|(i, t)| *i != me && table.points(**t) > target)
			.fold(0, |m, (i, _)| m | (1 << i));
		Some(Self {
			table,
			team,
			extra,
			target,
			own_division: team.division().index() % 4,
			conf,
			me,
			divisions,
			forced,
		})
	}

	fn solve(&self) -> Option<Scenario> {
		self.as_division_winner().or_else(|| self.search_wild_card())
	}

	fn as_division_winner(&self) -> Option<Scenario> {
		if self.forced & self.divisions[self.own_division] != 0 {
			return None;
		}
		let others: Vec<usize> = (0..self.divisions.len()).filter(|d| *d != self.own_division).collect();
		let mut subsets: Vec<u32> = (0..1u32 << others.len()).collect();
		subsets.sort_by_key(|s| (s.count_ones(), *s));
		for subset in subsets {
			let free = others
				.iter()
				.enumerate()
				.filter(|(bit, _)| subset & (1 << bit) != 0)
				.fold(0, |m, (_, d)| m | self.divisions[*d]);
			if self.forced & !free & !self.divisions[self.own_division] != 0 {
				continue;
			}
			if let Placed::Yes(winners) = self.place(free) {
				let seed = 1 + u8::try_from(subset.count_ones()).unwrap_or(u8::MAX);
				return Some(self.scenario(seed, &winners, free));
			}
		}
		None
	}

	/// The seed a wild card gets with `free` allowed above it.
	fn wild_card_seed(&self, free: Mask) -> u8 {
		let extra_teams: u32 = self.divisions.iter().map(|d| (free & d).count_ones().saturating_sub(1)).sum();
		5u8.saturating_add(u8::try_from(extra_teams).unwrap_or(u8::MAX))
	}

	fn search_wild_card(&self) -> Option<Scenario> {
		let mut best: Option<Scenario> = None;
		let mut visited = HashSet::new();
		let mut stack = vec![self.forced];
		while let Some(free) = stack.pop() {
			let seed = self.wild_card_seed(free);
			let bound = best.as_ref().map_or(LAST_SEED + 1, |s| s.seed);
			if seed >= bound || !visited.insert(free) {
				continue;
			}
			match self.place(free) {
				Placed::Yes(winners) => best = Some(self.scenario(seed, &winners, free)),
				Placed::No(cut) => {
					if seed.saturating_add(self.frees_still_needed(free, cut)) >= bound {
						continue;
					}
					// Highest position first onto the stack, so the lowest pops first.
					for i in (0..self.conf.len()).rev().filter(|i| *i != self.me && cut & (1 << i) != 0) {
						stack.push(free | (1 << i));
					}
				}
			}
		}
		best
	}

	/// A lower bound on the seeds still to lose from `free`, given `cut`: the
	/// capped teams on the source side of the failed placement's minimum cut
	/// (the asked team among them when it is there). Their games among
	/// themselves hand out more points than their caps hold, by `over`. A
	/// feasible superset must free some of them; freeing `t` takes its games
	/// inside the cut out of the count and its cap out of the room, so it
	/// recovers at most `2 * games_t - cap_t`. At least `k` frees are needed,
	/// `k` the fewest of the largest recoveries that cover `over` (none can:
	/// no superset is feasible). A division with no free team yet takes its
	/// first one at no cost.
	fn frees_still_needed(&self, free: Mask, cut: Mask) -> u8 {
		let members: Vec<usize> = (0..self.conf.len()).filter(|i| cut & (1 << i) != 0).collect();
		let inside = |a: usize, b: usize| {
			let (a, b) = (self.conf[a], self.conf[b]);
			self.table.pending().iter().filter(|p| (p.home == a && p.away == b) || (p.home == b && p.away == a)).count()
		};
		let mut games = 0i64;
		let mut room = 0i64;
		// Per member that can be freed: (its division, what freeing it recovers).
		let mut recover: Vec<(usize, i64)> = Vec::new();
		for &i in &members {
			let g: usize = members.iter().filter(|j| **j != i).map(|&j| inside(i, j)).sum();
			let g = i64::try_from(g).unwrap_or(i64::MAX);
			let cap = i64::from(self.cap(i).unwrap_or(0));
			games += g;
			room += cap;
			if i != self.me {
				let d = self.divisions.iter().position(|d| d & (1 << i) != 0).unwrap_or(0);
				recover.push((d, 2 * g - cap));
			}
		}
		// Each game inside was counted from both ends.
		let mut over = games - room;
		// The cheapest cover: each untouched division's best recovery is free
		// (at most one per division), then the largest of the rest, one seed each.
		recover.sort_unstable_by(|a, b| b.1.cmp(&a.1));
		let mut paid = Vec::new();
		let mut used = 0u8;
		for (d, r) in recover {
			if free & self.divisions[d] == 0 && used & (1 << d) == 0 {
				used |= 1 << d;
				over -= r.max(0);
			} else {
				paid.push(r);
			}
		}
		let mut k = 0u8;
		for r in paid {
			if over <= 0 || r <= 0 {
				break;
			}
			over -= r;
			k += 1;
		}
		if over > 0 {
			LAST_SEED + 1
		} else {
			k
		}
	}

	fn capped(&self, team: Team, free: Mask) -> bool {
		self.conf.iter().position(|t| *t == team).is_some_and(|i| free & (1 << i) == 0)
	}

	fn cap(&self, i: usize) -> Option<u32> {
		if i == self.me {
			return Some(u32::from(self.extra) * 2);
		}
		// A remaining game hands out a whole win, so odd room left by a banked
		// tie is half a win nobody can take: round it down. Every capacity is
		// then even, the flow moves points in pairs, and no game is split.
		self.target.checked_sub(self.table.points(self.conf[i])).map(|room| u32::from(room) & !1)
	}

	/// Can every pending game be placed with only `free` (and the other
	/// conference) above the asked team?
	fn place(&self, free: Mask) -> Placed {
		let n = self.conf.len();
		let (source, sink) = (0, 1);
		let team_node = |i: usize| 2 + i;
		let games: Vec<usize> = (0..self.table.pending().len())
			.filter(|&g| {
				let p = self.table.pending()[g];
				self.capped(p.home, free) && self.capped(p.away, free)
			})
			.collect();
		let mut net = Network::new(2 + n + games.len());
		for i in (0..n).filter(|i| free & (1 << i) == 0) {
			let Some(cap) = self.cap(i) else {
				return Placed::No(1 << i);
			};
			net.add(team_node(i), sink, cap);
		}
		let pos = |t: Team| self.conf.iter().position(|c| *c == t).unwrap_or(0);
		let mut home_edges = Vec::with_capacity(games.len());
		for (k, &g) in games.iter().enumerate() {
			let p = self.table.pending()[g];
			let node = 2 + n + k;
			net.add(source, node, 2);
			home_edges.push((net.add(node, team_node(pos(p.home)), 2), net.add(node, team_node(pos(p.away)), 2)));
		}
		let need = u32::try_from(games.len()).unwrap_or(u32::MAX) * 2;
		let flow = net.max_flow(source, sink);
		if flow < need {
			let side = net.reachable(source);
			let cut = (0..n).filter(|&i| free & (1 << i) == 0 && side[team_node(i)]).fold(0, |m, i| m | (1 << i));
			return Placed::No(cut);
		}
		let winners = games
			.iter()
			.zip(home_edges)
			.map(|(&g, (home, _))| {
				let p = self.table.pending()[g];
				(g, if net.flow_on(home) == 2 { p.home } else { p.away })
			})
			.collect();
		Placed::Yes(winners)
	}

	/// The whole rest of the season for a placement: the flow decides games
	/// between capped teams; a free team or the other conference takes every
	/// game against a capped one; then the asked team takes wins from its
	/// remaining losses, in schedule order, until it has exactly `extra`.
	fn scenario(&self, seed: u8, placed: &[(usize, Team)], free: Mask) -> Scenario {
		let pending = self.table.pending();
		let mut winner: Vec<Team> = pending.iter().map(|p| if self.capped(p.home, free) { p.away } else { p.home }).collect();
		for &(g, w) in placed {
			winner[g] = w;
		}
		let mut wins = winner.iter().filter(|w| **w == self.team).count();
		for (g, p) in pending.iter().enumerate() {
			if wins >= usize::from(self.extra) {
				break;
			}
			if (p.home == self.team || p.away == self.team) && winner[g] != self.team {
				winner[g] = self.team;
				wins += 1;
			}
		}
		let mut rest = [Record::default(); 32];
		for (p, w) in pending.iter().zip(winner) {
			rest[w.index()].w += 1;
			rest[(if w == p.home { p.away } else { p.home }).index()].l += 1;
		}
		Scenario { seed, rest }
	}
}

#[cfg(test)]
mod tests;
