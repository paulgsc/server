//! The output, contract version 2.
//!
//! `paulgsc/some-ui`'s `packages/ui/playoff-delta/src/lib/story.ts`, field
//! for field. That zod schema is the contract; these types
//! only serialize to it. A field added there is added here in the same pair of
//! changes, and `tests/evaluate.rs` checks the keys against a hand copy of
//! the schema (invariant PD3 in the README).

use crate::league::Team;
use serde::{Serialize, Serializer};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct WinLoss {
	pub w: u8,
	pub l: u8,
}

/// The headline of a paths row: the fewest more wins that still get in, and
/// the best seed with exactly that many.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Claim {
	In {
		#[serde(rename = "in")]
		is_in: True,
		wins: u8,
		#[serde(rename = "extraWins")]
		extra_wins: u8,
		seed: u8,
	},
	Out {
		#[serde(rename = "in")]
		is_in: False,
		wins: u8,
	},
}

/// `true`, as a type, so `Claim::In` cannot serialize `"in": false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct True;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct False;

impl Serialize for True {
	fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
		s.serialize_bool(true)
	}
}

impl Serialize for False {
	fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
		s.serialize_bool(false)
	}
}

impl Claim {
	#[must_use]
	pub const fn new(wins: u8, path: Option<(u8, u8)>) -> Self {
		match path {
			Some((extra_wins, seed)) => Self::In {
				is_in: True,
				wins,
				extra_wins,
				seed,
			},
			None => Self::Out { is_in: False, wins },
		}
	}

	/// Orders claims: fewer total wins needed, then a better seed; out is
	/// worst. Lower is better.
	#[must_use]
	pub const fn rank(self) -> (u16, u8) {
		match self {
			Self::In { wins, extra_wins, seed, .. } => (wins as u16 + extra_wins as u16, seed),
			Self::Out { .. } => (u16::MAX, u8::MAX),
		}
	}
}

/// A share of ways (0 to 1), written to four places: a hundredth of a
/// percentage point, below what a sampled share can tell apart.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Share(pub f64);

impl Serialize for Share {
	fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
		// `+ 0.0` turns a rounded -0.0 into 0.0.
		s.serialize_f64((self.0 * 10_000.0).round() / 10_000.0 + 0.0)
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Moved {
	Helped,
	Hurt,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Collapse {
	pub peak_win_probability: f64,
	pub led: bool,
}

/// What kind of game a result was, for the team the story follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
	/// The team's own game.
	Own,
	/// A conference team against the other conference.
	Exported,
	/// Two conference teams, neither of them the team.
	Inside,
}

/// One decided game that can matter to the team.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoryResult {
	pub winner: Team,
	pub loser: Team,
	pub kind: Kind,
	/// The share as it happened, minus the share had it gone the other way.
	pub leverage: Share,
	/// Did flipping it change the team's paths, and which way.
	pub moved: Option<Moved>,
	/// The loser's best moment in the game, when the feed has it.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub collapse: Option<Collapse>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Relation {
	Division,
	Rival,
	Conference,
	Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoryGame {
	pub opponent: Team,
	pub won: bool,
	pub relation: Relation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoryPaths {
	pub before: Vec<Option<u8>>,
	pub after: Vec<Option<u8>>,
	pub wins_before: u8,
	pub wins_after: u8,
}

/// Of the ways the rest of the season can go, the share in which the team
/// gets in, and how the week moved it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct StoryShare {
	pub before: Share,
	pub after: Share,
	/// The change the team's own result made (0 on a bye or a tie).
	pub own: Share,
	/// The rest of the change: everyone else's results.
	pub league: Share,
	/// Every way was counted, before and after, not a sample.
	pub exact: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Left {
	pub before: u16,
	pub after: u16,
}

/// A game next week, worth rooting for.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Next {
	pub home: Team,
	pub away: Team,
	pub root_for: Team,
	/// The share if `root_for` wins, minus the share if it loses.
	pub swing: Share,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Need {
	pub team: Team,
	pub rest: WinLoss,
	pub rival: bool,
	/// This way in lets the team finish above: `rest` is one choice.
	pub free: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoryPath {
	pub extra_wins: u8,
	pub seed: u8,
	pub needs: Vec<Need>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Bucket {
	pub team: Team,
	pub wins: u8,
	pub room: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pigeonhole {
	pub teams: Vec<Bucket>,
	pub games: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum DivisionRoute {
	Above { team: Team },
	Pigeonhole { teams: Vec<Bucket>, games: u16 },
}

/// A group that alone, with the teams already above, closes the wild card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WildCardPigeonhole {
	pub teams: Vec<Bucket>,
	pub games: u16,
	/// At least this many of them, besides one division winner per division,
	/// must finish above the team.
	pub must_pass: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WildCardRoute {
	pub above: Vec<Team>,
	/// When one group closes it; otherwise the proof is the search.
	pub pigeonhole: Option<WildCardPigeonhole>,
	/// Placements of who else finishes above that the search tried (0 when
	/// the teams already above close it).
	pub cases: u32,
}

/// Why one fewer final win can't get the team in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Boundary {
	/// The final wins that fall short.
	pub wins: u8,
	pub division: DivisionRoute,
	pub wild_card: WildCardRoute,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RivalGame {
	pub opponent: Team,
	pub won: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Shares {
	pub before: Share,
	pub after: Share,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RivalWeek {
	pub team: Team,
	pub game: Option<RivalGame>,
	pub before: Claim,
	pub after: Claim,
	pub share: Shares,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Collision {
	#[serde(rename_all = "camelCase")]
	RivalVsYou {
		home: Team,
		away: Team,
		winner: Team,
		rival: Team,
		worst_for_rival: Team,
		best_for_you: Team,
	},
	RivalVsRival {
		home: Team,
		away: Team,
		winner: Team,
	},
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Score {
	pub scored: u16,
	pub possible: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Points {
	pub week: Score,
	pub season: Score,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LowestBar {
	pub team: Team,
	pub wins: u8,
	pub seed: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Benchmark {
	pub last_season: Claim,
	pub lowest_bar: LowestBar,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoryWeek {
	pub week: u8,
	pub record: WinLoss,
	/// `None` on the team's bye.
	pub game: Option<StoryGame>,
	pub paths: StoryPaths,
	pub share: StoryShare,
	/// Games left in the league.
	pub pending: Left,
	/// Every decided game this week that can matter, most leverage first.
	pub results: Vec<StoryResult>,
	pub next: Vec<Next>,
	pub path: Option<StoryPath>,
	pub boundary: Option<Boundary>,
	pub rivals: Vec<RivalWeek>,
	pub collisions: Vec<Collision>,
	pub points: Points,
	pub benchmark: Benchmark,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorySeason {
	pub version: u8,
	pub label: String,
	pub season_length: u8,
	pub team: Team,
	pub rivals: Vec<Team>,
	pub names: BTreeMap<&'static str, &'static str>,
	pub conference: &'static str,
	/// The most ways a sampled share counts; fewer when every way is counted.
	pub ways: u32,
	pub weeks: Vec<StoryWeek>,
}
