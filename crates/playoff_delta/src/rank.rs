//! A finished season's seeding under the engine's standings model (PD1).
//!
//! A tie in the final standings goes to the team asked about, and no NFL
//! tiebreaker is applied. The solver answers "can this seed be reached?";
//! this answers "which seed is it?" for one set of final standings, and the
//! count ([`crate::count`]) and the solver's brute-force test both use it, so
//! the two cannot disagree about a season.

use crate::{league::Team, solver::LAST_SEED};

/// The seed `team` gets with final standings `points` (by [`Team::index`]),
/// ties going its way; `None` outside the playoffs.
#[must_use]
pub fn seed(points: &[u16; 32], team: Team) -> Option<u8> {
	let p = points[team.index()];
	let own = team.division().index();
	// Per division of the conference: teams strictly above `team`.
	let mut above = [0u8; 8];
	for t in Team::of_conference(team.conference()).filter(|t| *t != team) {
		if points[t.index()] > p {
			above[t.division().index()] += 1;
		}
	}
	if above[own] == 0 {
		// A division winner: behind each other division whose winner is above.
		let ahead = (0..8).filter(|d| *d != own && above[*d] > 0).count();
		return Some(1 + u8::try_from(ahead).unwrap_or(u8::MAX));
	}
	// A wild card: behind every team above it except one division winner per
	// division (the four winners always seed first).
	let ahead: u8 = above.iter().map(|a| a.saturating_sub(1)).sum();
	let seed = 5u8.saturating_add(ahead);
	(seed <= LAST_SEED).then_some(seed)
}
