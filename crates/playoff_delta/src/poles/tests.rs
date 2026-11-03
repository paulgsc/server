//! The poles against brute force: every outcome of a dozen games left, on
//! random fields (with banked ties, so half-win room is exercised), and on
//! random NFL tables for the worst seed, seeded by the rule itself (PD1).

use super::*;
use crate::{
	league::Team,
	rank::seed as seed_of,
	table::{Record, Table},
	testing::{final_points, random_case, Lcg},
};

/// A random field: `n` entities with some banked points (odd for a tie),
/// `games` games among them.
fn random_field(rng: &mut Lcg, n: usize, games: usize) -> (Vec<u16>, Vec<(usize, usize)>) {
	let points = (0..n).map(|_| u16::try_from(rng.next(9) * 2 + u64::from(rng.next(5) == 0)).unwrap()).collect();
	let n64 = u64::try_from(n).unwrap();
	let games = (0..games)
		.map(|_| {
			let a = usize::try_from(rng.next(n64)).unwrap();
			let b = (a + 1 + usize::try_from(rng.next(n64 - 1)).unwrap()) % n;
			(a, b)
		})
		.collect();
	(points, games)
}

/// Final points for one outcome: bit `i` of `outcome` set when game `i`'s
/// second side won.
fn finals(points: &[u16], games: &[(usize, usize)], outcome: u32) -> Vec<u16> {
	let mut out = points.to_vec();
	for (i, &(a, b)) in games.iter().enumerate() {
		out[if outcome >> i & 1 == 0 { a } else { b }] += 2;
	}
	out
}

/// The fewest and most points `group`'s best member finishes with, over
/// every outcome of every game.
fn extremes(points: &[u16], games: &[(usize, usize)], group: &[usize]) -> (u16, u16) {
	(0u32..1 << games.len())
		.map(|o| {
			let f = finals(points, games, o);
			group.iter().map(|&m| f[m]).max().unwrap()
		})
		.fold((u16::MAX, 0), |(lo, hi), x| (lo.min(x), hi.max(x)))
}

/// A random group of at least one, in index order.
fn random_group(rng: &mut Lcg, n: usize) -> Vec<usize> {
	let group: Vec<usize> = (0..n).filter(|_| rng.next(3) != 0).collect();
	if group.is_empty() {
		vec![0]
	} else {
		group
	}
}

#[test]
fn the_floor_and_ceiling_are_the_extremes() {
	let mut rng = Lcg(0xf100);
	for case in 0..200 {
		let (points, games) = random_field(&mut rng, 6, 12);
		let field = Field { points: &points, games: &games };
		let group = random_group(&mut rng, 6);
		let (lo, hi) = extremes(&points, &games, &group);
		let floor = field.floor(&group).unwrap();
		assert_eq!(floor.points, lo, "case {case}: {group:?}");
		// No subject in the group: every game left can go to a member.
		let ceiling = field.ceiling(&group, None).unwrap();
		assert_eq!(ceiling.points, hi, "case {case}");
		assert!(group.contains(&ceiling.by));
		// The bloc forces the floor with its own games, and needs every member.
		let bloc = &floor.bloc;
		assert!(bloc.iter().all(|m| group.contains(m)), "case {case}");
		let own: Vec<(usize, usize)> = games.iter().copied().filter(|(a, b)| bloc.contains(a) && bloc.contains(b)).collect();
		assert_eq!(extremes(&points, &own, bloc).0, lo, "case {case}: {bloc:?}");
		assert_eq!(usize::from(floor.games), own.len());
		assert_eq!(floor.banked, bloc.iter().map(|&m| points[m]).sum::<u16>());
		for i in (0..bloc.len()).filter(|_| bloc.len() > 1) {
			let mut rest = bloc.clone();
			rest.remove(i);
			let rest_games: Vec<(usize, usize)> = own.iter().copied().filter(|(a, b)| rest.contains(a) && rest.contains(b)).collect();
			assert!(extremes(&points, &rest_games, &rest).0 < lo, "case {case}: {rest:?} still forces {lo}");
		}
	}
}

#[test]
fn held_members_stay_at_or_under_their_cap() {
	// As the story holds the division at its floor: several members, one cap.
	let mut rng = Lcg(0x4e1d);
	for case in 0..100 {
		let (points, games) = random_field(&mut rng, 6, 11);
		let field = Field { points: &points, games: &games };
		let group = random_group(&mut rng, 6);
		let held: Vec<usize> = group.iter().copied().filter(|_| rng.next(2) == 0).collect();
		let cap = held.iter().map(|&m| points[m]).max().unwrap_or(0) + 2 * u16::try_from(rng.next(4)).unwrap();
		let oracle = (0u32..1 << games.len())
			.filter_map(|o| {
				let f = finals(&points, &games, o);
				held.iter().all(|&m| f[m] <= cap).then(|| group.iter().map(|&m| f[m]).max().unwrap())
			})
			.min();
		let caps: Vec<(usize, u16)> = held.iter().map(|&m| (m, cap)).collect();
		assert_eq!(field.floor_held(&group, &caps), oracle, "case {case}");
	}
}

#[test]
fn winning_out_takes_the_subject_s_games_from_the_ceiling() {
	let points = [4, 6, 2];
	let games = [(0, 1), (1, 2), (1, 2), (0, 2)];
	let field = Field { points: &points, games: &games };
	// 1 reaches 12 winning all three; with subject 2 winning out, 8.
	assert_eq!(field.ceiling(&[0, 1], None), Some(Ceiling { points: 12, by: 1 }));
	assert_eq!(field.ceiling(&[0, 1], Some(2)), Some(Ceiling { points: 8, by: 1 }));
}

#[test]
fn a_round_robin_of_three_holds_each_to_two() {
	// Week 0 of a division: three rivals, each pair meeting twice.
	let points = [0; 3];
	let games = [(0, 1), (1, 0), (1, 2), (2, 1), (0, 2), (2, 0)];
	let field = Field { points: &points, games: &games };
	let floor = field.floor(&[0, 1, 2]).unwrap();
	assert_eq!((floor.points, floor.bloc, floor.games), (4, vec![0, 1, 2], 6));
}

#[test]
fn the_floor_only_rises_and_the_ceiling_only_falls() {
	// PD4, on fields: banking any game, either way, never lowers the floor
	// or raises the ceiling.
	let mut rng = Lcg(0xd4);
	for case in 0..100 {
		let (mut points, mut games) = random_field(&mut rng, 7, 14);
		let group = random_group(&mut rng, 7);
		let mut last = (0u16, u16::MAX);
		while !games.is_empty() {
			let field = Field { points: &points, games: &games };
			let now = (field.floor_points(&group).unwrap(), field.ceiling(&group, None).unwrap().points);
			assert!(now.0 >= last.0 && now.1 <= last.1, "case {case}: {last:?} then {now:?}");
			last = now;
			let (a, b) = games.remove(usize::try_from(rng.next(u64::try_from(games.len()).unwrap())).unwrap());
			points[if rng.next(2) == 0 { a } else { b }] += 2;
		}
	}
}

/// The worst seed over every outcome of the pending games, the subject's
/// own included.
fn worst_by_brute_force(table: &Table, records: &[Record; 32], team: Team) -> Option<u8> {
	let pending = table.pending();
	(0u32..1 << pending.len())
		.map(|o| seed_of(&final_points(records, pending, o), team))
		.try_fold(1, |worst, s| s.map(|s| worst.max(s)))
}

#[test]
fn the_worst_seed_is_the_worst_outcome() {
	let mut rng = Lcg(0x3eed);
	for _ in 0..30 {
		let (records, pending) = random_case(&mut rng, 11);
		let table = Table::from_parts(records, pending);
		let points: Vec<u16> = Team::ALL.iter().map(|&t| table.points(t)).collect();
		let games: Vec<(usize, usize)> = table.pending().iter().map(|p| (p.home.index(), p.away.index())).collect();
		let field = Field { points: &points, games: &games };
		for team in Team::ALL {
			let conference: Vec<usize> = Team::of_conference(team.conference()).map(Team::index).collect();
			let got = field.worst_seed(&conference, |e| Team::ALL[e].division().index(), team.index());
			assert_eq!(got, worst_by_brute_force(&table, &records, team), "{}", team.code());
		}
	}
}
