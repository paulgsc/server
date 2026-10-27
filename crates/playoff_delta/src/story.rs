//! The output, contract version 3.
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Stake {
	/// The share if the team wins next week minus the share if it loses,
	/// unsigned; 0 on its bye.
	pub own: Share,
	/// The share's standard deviation over the ways all of next week's
	/// results can go, the team's own game among them.
	pub spread: Share,
	/// Every way was counted, not a sample.
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
