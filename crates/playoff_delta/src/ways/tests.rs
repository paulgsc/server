//! A week's ways against brute force over small columns.

use super::*;
use crate::testing::Lcg;

/// A column of `games` games with shares from `rng`: a few slates share a
/// value, so ties are exercised.
fn column(rng: &mut Lcg, games: usize) -> Column {
	let share: Vec<f64> = (0..1usize << games).map(|_| f64::from(u32::try_from(rng.next(9)).unwrap()) / 8.0).collect();
	Column {
		games,
		halves: [share.clone(), share.clone()],
		share,
		exact: true,
	}
}

#[test]
fn a_price_is_the_best_with_the_wanted_side_minus_the_best_without() {
	let mut rng = Lcg(0x9A7);
	for _ in 0..50 {
		let c = column(&mut rng, 5);
		let Best { best, pieces } = c.best(|_| true);
		assert!((best - c.share.iter().copied().fold(0.0, f64::max)).abs() < 1e-12);
		for (j, piece) in pieces.iter().enumerate() {
			let side = |home: bool| (0..32).filter(|s| (s >> j & 1 == 1) == home).map(|s| c.share[s]).fold(0.0, f64::max);
			let (want, other) = (side(piece.want_home), side(!piece.want_home));
			assert!(want >= other, "game {j}: the wanted side is the better one");
			assert!((piece.price - (want - other)).abs() < 1e-12);
			// The best way wins every game the way it wants, but for free ones.
			assert!((want - best).abs() < 1e-12);
			assert!(piece.error.abs() < 1e-12, "a counted column has no error");
		}
	}
}

#[test]
fn a_held_game_costs_nothing_and_wants_what_happened() {
	let mut rng = Lcg(0x9A8);
	let c = column(&mut rng, 4);
	for home in [true, false] {
		let held = c.best(|s| (s & 1 == 1) == home);
		assert_eq!(held.pieces[0].want_home, home);
		assert!(held.pieces[0].price.abs() < 1e-12);
		let brute = (0..16).filter(|s| (s & 1 == 1) == home).map(|s| c.share[s]).fold(0.0, f64::max);
		assert!((held.best - brute).abs() < 1e-12);
	}
}

#[test]
fn the_road_runs_from_the_best_to_the_actual_share_and_never_rises() {
	let mut rng = Lcg(0x9A9);
	for round in 0..50 {
		let c = column(&mut rng, 5);
		let actual = usize::try_from(rng.next(32)).unwrap();
		let order = if round % 2 == 0 { vec![0, 1, 2, 3, 4] } else { vec![3, 0, 4, 2, 1] };
		let road = c.road(actual, &order);
		assert_eq!(road.best.len(), 6);
		assert!((road.best[0] - c.best(|_| true).best).abs() < 1e-12);
		assert!((road.best[5] - c.share[actual]).abs() < 1e-12);
		assert!((road.mean[5] - c.share[actual]).abs() < 1e-12);
		assert!(road.best.windows(2).all(|p| p[1] <= p[0]));
		// The mean of every way is the week's mean.
		let mean = c.share.iter().sum::<f64>() / 32.0;
		assert!((road.mean[0] - mean).abs() < 1e-12);
		// Each point keeps half the ways the last one did.
		for (p, row) in road.strip.iter().enumerate() {
			assert_eq!(row.iter().sum::<u32>(), 32 >> p);
		}
		// The actual way is in the last point's one way, in its bin.
		assert_eq!(road.strip[5].iter().filter(|&&n| n > 0).count(), 1);
	}
}

#[test]
fn a_rank_counts_the_ways_below_and_half_the_ties() {
	let c = Column {
		games: 2,
		share: vec![0.1, 0.5, 0.5, 0.9],
		halves: [vec![0.1, 0.5, 0.5, 0.9], vec![0.1, 0.5, 0.5, 0.9]],
		exact: true,
	};
	assert!((c.rank(3, |_| true) - 0.875).abs() < 1e-12);
	assert!((c.rank(1, |_| true) - 0.5).abs() < 1e-12);
	assert!((c.rank(0, |_| true) - 0.125).abs() < 1e-12);
	// Among the ways with game 0's home side winning: 0.5 and 0.9.
	assert!((c.rank(1, |s| s & 1 == 1) - 0.25).abs() < 1e-12);
}

#[test]
fn a_flat_week_puts_every_way_in_the_last_bin() {
	let c = Column {
		games: 1,
		share: vec![1.0, 1.0],
		halves: [vec![1.0, 1.0], vec![1.0, 1.0]],
		exact: true,
	};
	let road = c.road(0, &[0]);
	assert_eq!(road.strip[0][BINS - 1], 2);
	assert!(c.best(|_| true).pieces[0].price.abs() < 1e-12);
}

#[test]
fn a_price_read_off_disagreeing_halves_carries_their_gap() {
	let c = Column {
		games: 1,
		share: vec![0.40, 0.50],
		halves: [vec![0.40, 0.52], vec![0.40, 0.48]],
		exact: false,
	};
	let piece = c.best(|_| true).pieces[0];
	assert!(piece.want_home);
	assert!((piece.price - 0.10).abs() < 1e-12);
	assert!((piece.error - 0.02).abs() < 1e-12);
}
