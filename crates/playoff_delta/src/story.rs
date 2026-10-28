//! The output, contract version 4.
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
	/// The loser's best moment in the game, when the feed has it.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub collapse: Option<Collapse>,
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
	/// How far everyone else's results could have moved the share: its
	/// standard deviation over the ways they could have gone, the team's own
	/// result held as it happened.
	pub spread: Share,
	/// Every way was counted, before and after, not a sample.
	pub exact: bool,
}

/// What next week can do to the share, before it is played.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Stake {
	/// The team's opponent next week; `None` on its bye.
	pub opponent: Option<Team>,
	/// The share if the team wins next week minus the share if it loses,
	/// unsigned; 0 on its bye, or when its game cannot move the share.
	pub own: Share,
	/// The share's standard deviation over the ways all of next week's
	/// results can go, the team's own game among them.
	pub spread: Share,
	/// Every way was counted, not a sample.
	pub exact: bool,
	/// Next week's best way: its games with a side in the conference, in
	/// kickoff order, each with its piece.
	pub pieces: Vec<NextPiece>,
}

/// A game's place in the best way a week can go ([`crate::ways`]).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Piece {
	/// The side the best way needs.
	pub want: Team,
	/// The best share any way reaches with the game won by `want`, minus the
	/// best with it won by the other side.
	pub price: Share,
	/// Half the gap between the price read off two independent halves of the
	/// rests; 0 when every rest was counted.
	pub error: Share,
}

/// A game of next week, with its piece.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct NextPiece {
	pub home: Team,
	pub away: Team,
	#[serde(flatten)]
	pub piece: Piece,
}

/// One of the week's games that could move the team, with its piece.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct WayGame {
	pub home: Team,
	pub away: Team,
	pub kind: Kind,
	pub winner: Team,
	#[serde(flatten)]
	pub piece: Piece,
}

/// The week's ways read with the team's own result held as it happened.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Given {
	pub best: Share,
	pub rank: Share,
	/// Aligned with [`Ways::games`]; the own game's piece is its actual
	/// winner, at no price.
	pub pieces: Vec<Piece>,
}

/// Every way the week's results could have gone, each counted against the
/// same rests of the season: one table, so every derivation adds up.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Ways {
	/// The week's games with a side in the conference, in kickoff order.
	pub games: Vec<WayGame>,
	pub best: Share,
	pub worst: Share,
	pub actual: Share,
	/// Of the week's ways, the share that would have left the team worse
	/// than `actual`, ties counted half.
	pub rank: Share,
	/// `None` on a bye or a tied own game.
	pub given: Option<Given>,
	/// The best share still reachable before any result, then after each, in
	/// kickoff order: the first is `best`, the last `actual`.
	pub road: Vec<Share>,
	/// The mean share of the ways still possible, at the same points.
	pub mean: Vec<Share>,
	/// At the same points, the ways still possible per bin of
	/// [`crate::ways::BINS`] over `[worst, best]`.
	pub strip: Vec<Vec<u32>>,
	/// Every rest was counted, not a sample.
	pub exact: bool,
}

/// A team's own result in a week.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Played {
	Won,
	Lost,
	Tie,
	Bye,
}

/// A medal's piece: whether the week gave the side its best way wanted.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct MedalPiece {
	pub home: Team,
	pub away: Team,
	pub want: Team,
	pub price: Share,
	pub made: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Medal {
	pub team: Team,
	/// 1 + the eligible teams with a strictly higher rank.
	pub place: u8,
	pub rank: Share,
	pub best: Share,
	pub worst: Share,
	pub actual: Share,
	pub game: Played,
	/// Its dearest pieces, at most three, dearest first.
	pub pieces: Vec<MedalPiece>,
}

/// Every team's actual week ranked among its own ways.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Podium {
	/// Every eligible team placed 1 to 3, best first.
	pub medals: Vec<Medal>,
	/// The team's own place among the eligible; `None` when not eligible.
	pub place: Option<u8>,
	/// Teams whose week could move their share by at least
	/// [`crate::evaluate::ELIGIBLE`].
	pub eligible: u8,
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
	/// `None` once no game is left.
	pub stake: Option<Stake>,
	/// Proven: every way the rest of the season can go gets the team in
	/// (PD1). `false` is "not proven", not "can miss".
	pub clinched: bool,
	pub ways: Ways,
	pub podium: Podium,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorySeason {
	pub version: u8,
	pub label: String,
	pub season_length: u8,
	pub team: Team,
	pub names: BTreeMap<&'static str, &'static str>,
	pub conference: &'static str,
	/// The most ways a sampled share counts; fewer when every way is counted.
	pub ways: u32,
	pub weeks: Vec<StoryWeek>,
}
