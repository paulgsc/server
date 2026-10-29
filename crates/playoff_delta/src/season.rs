//! A regular season as the evaluator reads it: every game, played or not.
//!
//! This is the one input. A feed adapter (outside this crate) builds it; the
//! evaluator never fetches, never guesses a schedule, and refuses a season
//! whose shape it cannot trust rather than computing paths from a hole.

use crate::league::Team;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Score {
	pub home: u16,
	pub away: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
	pub week: u8,
	pub home: Team,
	pub away: Team,
	/// `None` until the game is final.
	pub score: Option<Score>,
	/// When it kicks off, as any key that sorts the week's games in the
	/// order they start (the ESPN adapter writes minutes since the Unix
	/// epoch). `None` when the feed has no time; such games sort last.
	pub kickoff: Option<u64>,
}

/// Who a final game went to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
	Won(Team),
	Tied,
}

impl Game {
	#[must_use]
	pub fn outcome(&self) -> Option<Outcome> {
		let s = self.score?;
		Some(match s.home.cmp(&s.away) {
			std::cmp::Ordering::Greater => Outcome::Won(self.home),
			std::cmp::Ordering::Less => Outcome::Won(self.away),
			std::cmp::Ordering::Equal => Outcome::Tied,
		})
	}

	#[must_use]
	pub fn involves(&self, team: Team) -> bool {
		self.home == team || self.away == team
	}

	/// The other side of the game from `team`.
	#[must_use]
	pub fn opponent(&self, team: Team) -> Team {
		if self.home == team {
			self.away
		} else {
			self.home
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeasonError {
	/// A team plays itself.
	SelfGame { week: u8, team: Team },
	/// A team appears in two games in one week.
	DoubleBooked { week: u8, team: Team },
	/// Teams play different numbers of games, so "the rest of the season"
	/// would mean different things for different teams.
	UnevenSchedule { team: Team, games: usize, expected: usize },
	/// A later game is final while an earlier week still has one pending.
	/// Evaluating "after week n" needs every game through n.
	ResultsOutOfOrder { pending_week: u8, final_week: u8 },
}

impl fmt::Display for SeasonError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::SelfGame { week, team } => write!(f, "week {week}: {} plays itself", team.code()),
			Self::DoubleBooked { week, team } => write!(f, "week {week}: {} plays twice", team.code()),
			Self::UnevenSchedule { team, games, expected } => write!(f, "{} has {games} games, the rest of the league {expected}", team.code()),
			Self::ResultsOutOfOrder { pending_week, final_week } => write!(f, "week {final_week} has a final while week {pending_week} still has a game pending"),
		}
	}
}

impl std::error::Error for SeasonError {}

/// A checked season: every team plays the same number of games, at most once
/// a week, and results arrive in week order.
#[derive(Debug, Clone)]
pub struct Season {
	games: Vec<Game>,
	length: usize,
}

impl Season {
	/// # Errors
	/// Any [`SeasonError`]: the evaluator's answers are only as good as the
	/// schedule, so a malformed one is refused here, not half-used later.
	pub fn new(mut games: Vec<Game>) -> Result<Self, SeasonError> {
		games.sort_by_key(|g| (g.week, g.home, g.away));
		let mut seen = std::collections::HashSet::new();
		let mut count = [0usize; 32];
		for g in &games {
			if g.home == g.away {
				return Err(SeasonError::SelfGame { week: g.week, team: g.home });
			}
			for t in [g.home, g.away] {
				if !seen.insert((g.week, t)) {
					return Err(SeasonError::DoubleBooked { week: g.week, team: t });
				}
				count[t.index()] += 1;
			}
		}
		let length = count[0];
		if let Some(t) = Team::ALL.into_iter().find(|t| count[t.index()] != length) {
			return Err(SeasonError::UnevenSchedule {
				team: t,
				games: count[t.index()],
				expected: length,
			});
		}
		if let Some(pending) = games.iter().find(|g| g.score.is_none()) {
			if let Some(late) = games.iter().find(|g| g.score.is_some() && g.week > pending.week) {
				return Err(SeasonError::ResultsOutOfOrder {
					pending_week: pending.week,
					final_week: late.week,
				});
			}
		}
		Ok(Self { games, length })
	}

	/// Games per team.
	#[must_use]
	pub const fn length(&self) -> usize {
		self.length
	}

	#[must_use]
	pub fn games(&self) -> &[Game] {
		&self.games
	}

	/// The last week whose every game is final (0 before any).
	#[must_use]
	pub fn complete_through(&self) -> u8 {
		let last = self.games.last().map_or(0, |g| g.week);
		self.games.iter().find(|g| g.score.is_none()).map_or(last, |g| g.week.saturating_sub(1))
	}

	/// `team`'s game in `week`, or `None` on its bye.
	#[must_use]
	pub fn game_of(&self, team: Team, week: u8) -> Option<&Game> {
		self.games.iter().find(|g| g.week == week && g.involves(team))
	}

	/// Every game in `week`.
	pub fn week(&self, week: u8) -> impl Iterator<Item = &Game> {
		self.games.iter().filter(move |g| g.week == week)
	}
}
