//! The fixed set of scenarios a game-week bite is said about (Game Week Bites,
//! paulgsc/some-ui#2074, story GWB1).
//!
//! A bite is commentary on one scenario, drawn from a closed set known at
//! compile time: before kickoff, during the game, after the final, and the
//! owner's own watching. The scenario engine (GWB3) compiles against
//! [`Scenario`], and the phone's recorder reads a copy of [`catalog`]'s JSON
//! (`dump-game-scenarios`), so both agree on every id and on
//! [`CATALOG_VERSION`]. Changing the set is a release: the README says how.

use serde::Serialize;

/// The version of the set. Every bite and every weekly bundle records it.
pub const CATALOG_VERSION: u32 = 1;

/// When in a game week a scenario belongs. Also the first segment of its id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
	/// Before kickoff.
	Pre,
	/// During the game.
	In,
	/// After the final.
	Post,
	/// The owner, watching or not.
	You,
}

/// The kind of moment a scenario can be happening in.
///
/// The scenario engine (GWB3) writes its rule for each kind against this
/// key. The engine decides from
/// the play log and the clock whether such a moment is now; this crate only
/// names the kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Moment {
	/// From the mode's pregame window opening until kickoff.
	Pregame,
	/// Around kickoff, once the toss is known.
	Kickoff,
	/// Each team's first possession.
	OpeningDrive,
	/// A score that changes who leads.
	LeadChange,
	/// Third down with 7 or more yards to go.
	ThirdAndLong,
	/// Fourth down.
	FourthDown,
	/// The try after a touchdown.
	Try,
	/// Any offensive snap.
	Snap,
	/// A possession inside the opponent's 20.
	RedZone,
	/// The last two minutes of either half.
	TwoMinute,
	/// A play that changed possession.
	Turnover,
	/// A flag on a play.
	Penalty,
	/// A sack.
	Sack,
	/// A gain of 20 yards or more.
	BigPlay,
	/// A field goal or extra point attempt.
	Kick,
	/// A player down on the field.
	Injury,
	/// After the game is final.
	Final,
	/// Any time during or after a game of the week.
	Watching,
}

/// One scenario's entry in the set, as the dump writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Entry {
	/// Stable, dotted: `<phase>.<name>`, e.g. `in.third-long`.
	pub id: &'static str,
	pub phase: Phase,
	/// The short name, as the owner reads it.
	pub name: &'static str,
	/// What the scenario covers, in one sentence.
	pub covers: &'static str,
	/// What home can later fill in about how it came out, as kebab-case
	/// slugs from our side. Never empty.
	pub outcomes: &'static [&'static str],
	pub moment: Moment,
}

/// The set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scenario {
	Weather,
	InjuryReport,
	Venue,
	CoinToss,
	Stakes,
	OpeningDrive,
	FellBehind,
	ThirdAndLong,
	FourthDown,
	TwoPoint,
	PlayCall,
	RedZone,
	TwoMinute,
	Turnover,
	Penalty,
	Sack,
	BigPlay,
	Kick,
	Injury,
	Result,
	Paths,
	Pattern,
	CaughtIt,
	SatThrough,
	MissedIt,
	DodgedIt,
	TunedInLate,
}

impl Scenario {
	/// Every scenario, in the order the legend lists them.
	pub const ALL: [Self; 27] = [
		Self::Weather,
		Self::InjuryReport,
		Self::Venue,
		Self::CoinToss,
		Self::Stakes,
		Self::OpeningDrive,
		Self::FellBehind,
		Self::ThirdAndLong,
		Self::FourthDown,
		Self::TwoPoint,
		Self::PlayCall,
		Self::RedZone,
		Self::TwoMinute,
		Self::Turnover,
		Self::Penalty,
		Self::Sack,
		Self::BigPlay,
		Self::Kick,
		Self::Injury,
		Self::Result,
		Self::Paths,
		Self::Pattern,
		Self::CaughtIt,
		Self::SatThrough,
		Self::MissedIt,
		Self::DodgedIt,
		Self::TunedInLate,
	];

	/// The scenario's entry in the set.
	#[must_use]
	#[allow(clippy::too_many_lines)] // one arm per scenario: the set itself
	pub const fn entry(self) -> Entry {
		const fn e(id: &'static str, phase: Phase, name: &'static str, covers: &'static str, outcomes: &'static [&'static str], moment: Moment) -> Entry {
			Entry {
				id,
				phase,
				name,
				covers,
				outcomes,
				moment,
			}
		}
		use Moment as M;
		use Phase::{In, Post, Pre, You};
		match self {
			Self::Weather => e(
				"pre.weather",
				Pre,
				"Weather",
				"What the forecast does to the game plan.",
				&["rain", "snow", "wind", "clear"],
				M::Pregame,
			),
			Self::InjuryReport => e(
				"pre.injury-report",
				Pre,
				"Injury report",
				"Who is out or limited, and what that changes.",
				&["key-player-out", "questionable", "all-clear"],
				M::Pregame,
			),
			Self::Venue => e("pre.venue", Pre, "Home or road", "Where it is played.", &["home", "road", "neutral"], M::Pregame),
			Self::CoinToss => e(
				"pre.coin-toss",
				Pre,
				"Coin toss",
				"Who got the ball first, and the choice made.",
				&["won-received", "won-deferred", "lost-received", "lost-deferred"],
				M::Kickoff,
			),
			Self::Stakes => e(
				"pre.stakes",
				Pre,
				"What it means for the paths",
				"Playoff Delta before kickoff: which paths this game keeps open.",
				&["must-win", "can-afford-it"],
				M::Pregame,
			),
			Self::OpeningDrive => e(
				"in.first-drive",
				In,
				"Opening drive",
				"The first possession, and whether it ended in points.",
				&["scored", "stalled"],
				M::OpeningDrive,
			),
			Self::FellBehind => e(
				"in.fell-behind",
				In,
				"Fell behind",
				"The score swung against us.",
				&["fell-behind", "took-the-lead-back"],
				M::LeadChange,
			),
			Self::ThirdAndLong => e(
				"in.third-long",
				In,
				"Third and long",
				"Third down with 7 or more to go.",
				&["converted", "stopped"],
				M::ThirdAndLong,
			),
			Self::FourthDown => e(
				"in.fourth-down",
				In,
				"Fourth down",
				"The call on fourth down, and how it went.",
				&["went-for-it", "field-goal", "punted"],
				M::FourthDown,
			),
			Self::TwoPoint => e(
				"in.two-point",
				In,
				"Two or one",
				"After a touchdown: try for two, or kick.",
				&["went-for-two", "kicked"],
				M::Try,
			),
			Self::PlayCall => e(
				"in.play-call",
				In,
				"Run or throw",
				"The play call in a moment that called for one.",
				&["ran", "threw-short", "threw-deep"],
				M::Snap,
			),
			Self::RedZone => e("in.red-zone", In, "Red zone", "A trip inside the 20.", &["touchdown", "field-goal", "empty"], M::RedZone),
			Self::TwoMinute => e(
				"in.two-minute",
				In,
				"Two-minute drill",
				"The end of a half, against the clock.",
				&["scored", "ran-out-of-time"],
				M::TwoMinute,
			),
			Self::Turnover => e(
				"in.turnover",
				In,
				"Turnover",
				"The ball changed hands.",
				&["interception", "fumble", "on-downs"],
				M::Turnover,
			),
			Self::Penalty => e(
				"in.penalty",
				In,
				"Penalty",
				"A flag, or one that should have been.",
				&["for-us-called", "for-us-missed", "against-us-called", "against-us-missed"],
				M::Penalty,
			),
			Self::Sack => e("in.sack", In, "Sack", "The quarterback brought down behind the line.", &["took-one", "got-one"], M::Sack),
			Self::BigPlay => e("in.big-play", In, "Big play", "A gain of 20 yards or more.", &["for-us", "against-us"], M::BigPlay),
			Self::Kick => e("in.kick", In, "Kick", "A field goal or extra point.", &["made", "missed", "blocked"], M::Kick),
			Self::Injury => e("in.injury", In, "Injury", "Someone went down during the game.", &["out", "came-back"], M::Injury),
			Self::Result => e("post.result", Post, "The result", "The final.", &["won", "lost", "tied"], M::Final),
			Self::Paths => e(
				"post.paths",
				Post,
				"What it did to the paths",
				"Playoff Delta after the final: what the result opened or closed.",
				&["paths-kept", "paths-lost"],
				M::Final,
			),
			Self::Pattern => e(
				"post.pattern",
				Post,
				"Same story again",
				"Whether the game repeated something seen before.",
				&["fits-the-pattern", "breaks-it"],
				M::Final,
			),
			Self::CaughtIt => e(
				"you.caught-it",
				You,
				"Watched, and it went our way",
				"You were there for it.",
				&["result", "stretch"],
				M::Watching,
			),
			Self::SatThrough => e(
				"you.sat-through",
				You,
				"Watched, and it went against us",
				"You saw all of it.",
				&["result", "stretch"],
				M::Watching,
			),
			Self::MissedIt => e(
				"you.missed-it",
				You,
				"Didn't watch, and missed it going our way",
				"It went our way, maybe by a lot, and you braced yourself for nothing.",
				&["result", "stretch"],
				M::Watching,
			),
			Self::DodgedIt => e(
				"you.dodged-it",
				You,
				"Didn't watch, and dodged it going against us",
				"Not watching spared you.",
				&["result", "stretch"],
				M::Watching,
			),
			Self::TunedInLate => e(
				"you.tuned-in-late",
				You,
				"Tuned in late",
				"You came in when it looked lost, and caught the best stretch instead.",
				&["result", "stretch"],
				M::Watching,
			),
		}
	}

	/// The scenario's id, e.g. `in.third-long`.
	#[must_use]
	pub const fn id(self) -> &'static str {
		self.entry().id
	}

	/// The scenario an id names in this version of the set, if any.
	#[must_use]
	pub fn from_id(id: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|s| s.id() == id)
	}
}

/// The whole set, as `dump-game-scenarios` writes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Catalog {
	pub version: u32,
	pub scenarios: Vec<Entry>,
}

/// The set at [`CATALOG_VERSION`].
#[must_use]
pub fn catalog() -> Catalog {
	Catalog {
		version: CATALOG_VERSION,
		scenarios: Scenario::ALL.into_iter().map(Scenario::entry).collect(),
	}
}

#[cfg(test)]
mod tests;
