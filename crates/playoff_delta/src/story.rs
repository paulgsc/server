//! The output, contract version 1.
//!
//! `paulgsc/some-ui`'s `packages/ui/playoff-delta/src/lib/story.ts`, field
//! for field. That zod schema is the contract; these types
//! only serialize to it. A field added there is added here in the same pair of
//! changes, and `tests/evaluate.rs` checks the keys against a hand copy of
//! the schema (invariant PD3 in the README).

use crate::league::Team;
use serde::Serialize;
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
	fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
		s.serialize_bool(true)
	}
}

impl Serialize for False {
	fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
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
	/// worst. Lower is better. The client's `floorKey`, without its scale.
	#[must_use]
	pub const fn rank(self) -> (u16, u8) {
		match self {
			Self::In { wins, extra_wins, seed, .. } => (wins as u16 + extra_wins as u16, seed),
			Self::Out { .. } => (u16::MAX, u8::MAX),
		}
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

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoryResult {
	pub team: Team,
	pub opponent: Team,
	pub won: bool,
	pub your_way: Option<bool>,
	pub moved: Option<Moved>,
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

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Templates {
	pub division: Vec<StoryResult>,
	pub rivals: Vec<StoryResult>,
	pub above: Vec<StoryResult>,
	pub exported: Vec<StoryResult>,
	pub spoilers: Vec<StoryResult>,
	pub forced: Vec<StoryResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Need {
	pub team: Team,
	pub rest: WinLoss,
	pub rival: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoryPath {
	pub extra_wins: u8,
	pub seed: u8,
	pub needs: Vec<Need>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RivalGame {
	pub opponent: Team,
	pub won: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RivalWeek {
	pub team: Team,
	pub game: RivalGame,
	pub before: Claim,
	pub after: Claim,
	pub worst: Claim,
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
	pub game: StoryGame,
	pub paths: StoryPaths,
	pub templates: Templates,
	pub path: Option<StoryPath>,
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
	pub weeks: Vec<StoryWeek>,
}
