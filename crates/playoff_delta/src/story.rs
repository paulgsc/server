//! The output, contract version 6.
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

/// Wins, from standings points (a win 2, a tie 1): a banked tie is half a win.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Wins(pub u16);

impl Serialize for Wins {
	fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
		s.serialize_f64(f64::from(self.0) / 2.0)
	}
}

/// A change in wins, from a change in points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Steps(pub i32);

impl Serialize for Steps {
	fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
		s.serialize_f64(f64::from(self.0) / 2.0)
	}
}

/// What kind of game a result is, for the team the story follows.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Relation {
	Division,
	Conference,
	Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoryGame {
	pub opponent: Team,
	pub won: bool,
	pub relation: Relation,
}

/// The team's own wins: losing every game left, and winning every one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Range {
	pub low: Wins,
	pub high: Wins,
}

/// Rivals whose games among themselves force a floor on their own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Bloc {
	pub teams: Vec<Team>,
	pub banked: Wins,
	/// Games they have left against each other.
	pub games: u16,
}

/// The most wins a rival can still reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Ceiling {
	/// Winning every game it has left, ours included.
	pub lose: Wins,
	/// The same, with the team winning out: its games against us are losses.
	pub win: Wins,
	/// The first rival, in division order, that reaches `lose`.
	pub team: Team,
}

/// One group of rivals (our division's, or our conference's) at one table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Group {
	/// The best case: with the team winning out, the fewest wins the
	/// group's best rival can be held to.
	pub floor: Wins,
	pub bloc: Bloc,
	pub ceiling: Ceiling,
}

/// The best and worst seed the team can still finish with; `None` is out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Seeds {
	/// Winning out.
	pub best: Option<u8>,
	/// Losing out, with as many conference teams above as can be.
	pub worst: Option<u8>,
}

/// Every pole at one table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Poles {
	pub us: Range,
	pub division: Group,
	pub conference: Group,
	/// The conference's floor with the division held at its own floor.
	pub held: Wins,
	pub seeds: Seeds,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PolesPair {
	pub before: Poles,
	pub after: Poles,
}

/// Which pole a result moved the team toward, for one group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Cohort {
	/// It lowered the ceiling or the floor, or raised our wins.
	Best,
	/// It raised the floor or the ceiling, or cost us a win.
	Worst,
	/// Two of the group's rivals: one of them had to win it.
	Forced,
	/// Neither pole moved.
	Idle,
}

/// What a result did to one group's poles, against the same week with that
/// result the other way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Move {
	pub cohort: Cohort,
	/// The floor as it is, minus the floor had the result gone the other way.
	pub floor: Steps,
	/// The same, for the ceiling (`lose`).
	pub ceiling: Steps,
}

/// One of the week's results with a side in the conference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StoryResult {
	pub home: Team,
	pub away: Team,
	pub winner: Team,
	pub kind: Kind,
	/// `None` when neither side is us or a division rival.
	pub division: Option<Move>,
	pub conference: Move,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Left {
	pub before: u16,
	pub after: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoryWeek {
	pub week: u8,
	pub record: WinLoss,
	/// `None` on the team's bye.
	pub game: Option<StoryGame>,
	/// Games left in the league.
	pub pending: Left,
	pub poles: PolesPair,
	/// In kickoff order; a tied game is none of them.
	pub results: Vec<StoryResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorySeason {
	pub version: u8,
	pub label: String,
	pub season_length: u8,
	pub team: Team,
	pub names: BTreeMap<&'static str, &'static str>,
	pub conference: &'static str,
	/// The team's division rivals.
	pub rivals: Vec<Team>,
	pub weeks: Vec<StoryWeek>,
}
