//! The count against brute force: every outcome of every pending game, seeded
//! by the same rule ([`crate::rank::seed`]).

use super::*;
use crate::{
	table::{Pending, Record},
	testing::{random_case, Lcg},
};

/// The share of every outcome of `pending` in which `team` gets in, and per
/// game the share given each winner.
fn brute_force(records: &[Record; 32], pending: &[Pending], team: Team) -> (f64, Vec<(f64, f64)>) {
	let (mut ins, mut home) = (0u32, vec![(0u32, 0u32); pending.len()]);
	let total = 1u32 << pending.len();
	for outcome in 0..total {
		let mut points = records.map(Record::points);
		for (i, p) in pending.iter().enumerate() {
			points[(if outcome & (1 << i) == 0 { p.home } else { p.away }).index()] += 2;
		}
		let got = u32::from(seed(&points, team).is_some());
		ins += got;
		for (i, h) in home.iter_mut().enumerate() {
			if outcome & (1 << i) == 0 {
				h.0 += 1;
				h.1 += got;
			}
		}
	}
	let r = |a: u32, b: u32| f64::from(a) / f64::from(b);
	(r(ins, total), home.iter().map(|&(n, got)| (r(got, n), r(ins - got, total - n))).collect())
}

#[test]
fn counting_every_way_matches_brute_force() {
	let mut rng = Lcg(0xC0DE);
	for _ in 0..20 {
		let (records, pending) = random_case(&mut rng, 10);
		let table = Table::from_parts(records, pending.clone());
		for team in [Team::Sf, Team::Dal, Team::Kc, Team::Buf] {
			let all: Vec<usize> = (0..pending.len()).collect();
			let t = tally(&table, team, 1 << 12, &all);
			assert!(t.exact);
			let (share, given) = brute_force(&records, &pending, team);
			assert!((t.share - share).abs() < 1e-12, "{}: {} vs {share}", team.code(), t.share);
			for (i, p) in pending.iter().enumerate() {
				let inside = p.home.conference() == team.conference() || p.away.conference() == team.conference();
				match t.given[i] {
					Some((h, a)) => {
						assert!(inside);
						assert!((h - given[i].0).abs() < 1e-12 && (a - given[i].1).abs() < 1e-12, "{} game {i}", team.code());
					}
					// A game the conference does not play changes nothing.
					None => assert!(!inside && (given[i].0 - given[i].1).abs() < 1e-12),
				}
			}
		}
	}
}

#[test]
fn a_sample_lands_near_the_exact_share_and_repeats_itself() {
	let mut rng = Lcg(0xBEEF);
	let (records, pending) = random_case(&mut rng, 26);
	let table = Table::from_parts(records, pending.clone());
	let mut checked = 0;
	for team in Team::ALL {
		let n = pending
			.iter()
			.filter(|p| p.home.conference() == team.conference() || p.away.conference() == team.conference())
			.count();
		if !(16..=19).contains(&n) {
			continue;
		}
		checked += 1;
		let exact = tally(&table, team, 1 << n, &[]);
		let sampled = tally(&table, team, 20_000, &[]);
		assert!(exact.exact && !sampled.exact);
		// Four standard errors of a share near one half over 20,000 ways.
		assert!((exact.share - sampled.share).abs() < 0.015, "{}: {} vs {}", team.code(), exact.share, sampled.share);
		assert_eq!(sampled, tally(&table, team, 20_000, &[]));
	}
	assert!(checked > 0, "no team has 16 to 19 games to count");
}

/// A conference team's loss to the other conference never lowers the share
/// (conservation: the win leaves the conference), so given it, the share is
/// at least the share given the conference team won: counted every way, and
/// sampled, since both sides are counted on the same ways.
#[test]
fn a_loss_to_the_other_conference_never_hurts() {
	let mut rng = Lcg(0xF00D);
	for round in 0..20 {
		let (records, pending) = random_case(&mut rng, 12);
		let table = Table::from_parts(records, pending.clone());
		let all: Vec<usize> = (0..pending.len()).collect();
		for team in [Team::Sf, Team::Ne] {
			let t = tally(&table, team, if round % 2 == 0 { 1 << 12 } else { 300 }, &all);
			for (i, p) in pending.iter().enumerate() {
				let ours = |x: Team| x.conference() == team.conference() && x != team;
				let Some((home_won, away_won)) = t.given[i] else { continue };
				if ours(p.home) && !ours(p.away) && p.away != team {
					assert!(away_won >= home_won, "{} game {i}", team.code());
				}
				if ours(p.away) && !ours(p.home) && p.home != team {
					assert!(home_won >= away_won, "{} game {i}", team.code());
				}
			}
		}
	}
}
