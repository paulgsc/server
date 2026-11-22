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
pub const CATALOG_VERSION: u32 = 3;

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
	/// From the end of the 2nd quarter until the second half's kickoff.
	Halftime,
	/// A kickoff the kicking team tried to get back.
	OnsideKick,
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
	Halftime,
	OnsideKick,
}

impl Scenario {
	/// Every scenario, in the order the legend lists them.
	pub const ALL: [Self; 29] = [
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
		Self::Halftime,
		Self::OnsideKick,
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
			Self::Halftime => e(
				"in.halftime",
				In,
				"Halftime",
				"Where the first half left us, and what the second needs.",
				&["leading", "trailing", "tied"],
				M::Halftime,
			),
			Self::OnsideKick => e(
				"in.onside-kick",
				In,
				"Onside kick",
				"A kickoff the kicking team tried to get back.",
				&["ours", "theirs"],
				M::OnsideKick,
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

/// Which half of the game an in-game bite is in (#3048, D10): from the
/// quarter of the play it matched, or the game's quarter when it was said.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Half {
	First,
	Second,
	Overtime,
}

impl Half {
	pub const ALL: [Self; 3] = [Self::First, Self::Second, Self::Overtime];

	/// The half quarter `period` is in; none for a quarter ESPN never uses.
	#[must_use]
	pub const fn of_period(period: i64) -> Option<Self> {
		match period {
			1 | 2 => Some(Self::First),
			3 | 4 => Some(Self::Second),
			5.. => Some(Self::Overtime),
			_ => None,
		}
	}
}

/// How big a big play was (#3048, D8): 20 yards or more, 40, 60.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Size {
	Big,
	Huge,
	/// The kind you remember.
	Memorable,
}

impl Size {
	pub const ALL: [Self; 3] = [Self::Big, Self::Huge, Self::Memorable];

	/// The fewest yards of the size.
	#[must_use]
	pub const fn yards(self) -> i64 {
		match self {
			Self::Big => 20,
			Self::Huge => 40,
			Self::Memorable => 60,
		}
	}

	/// The size of a gain of `yards`; none under 20.
	#[must_use]
	pub fn of_yards(yards: i64) -> Option<Self> {
		Self::ALL.into_iter().rev().find(|size| yards >= size.yards())
	}
}

/// What kind of big play it was (#3048, D9), named for the side that gained.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
	Pass,
	Run,
	KickReturn,
	PuntReturn,
	InterceptionReturn,
	FumbleReturn,
}

impl Kind {
	pub const ALL: [Self; 6] = [Self::Pass, Self::Run, Self::KickReturn, Self::PuntReturn, Self::InterceptionReturn, Self::FumbleReturn];

	/// The smallest size a play of this kind counts as big from. A kickoff
	/// return of 20 to 39 yards is the ordinary one, out of the end zone to
	/// about the 25, so a kick return is big from 40.
	#[must_use]
	pub const fn least(self) -> Size {
		match self {
			Self::KickReturn => Size::Huge,
			Self::Pass | Self::Run | Self::PuntReturn | Self::InterceptionReturn | Self::FumbleReturn => Size::Big,
		}
	}

	/// The size of this kind's gain of `yards`, if it is a big play.
	#[must_use]
	pub fn size(self, yards: i64) -> Option<Size> {
		Size::of_yards(yards).filter(|size| *size >= self.least())
	}
}

/// A kind, as the dump writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct KindEntry {
	pub id: Kind,
	/// The smallest size it counts as big from.
	pub least: Size,
}

/// A size, as the dump writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct SizeEntry {
	pub id: Size,
	pub yards: i64,
}

/// The big-play algebra's closed parts (#3048), as the dump lists them.
///
/// A big play is a size, a kind, the side that gained, whether it scored,
/// and whether it was good for the side wanted; the last two are yes or no.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BigPlays {
	pub sizes: Vec<SizeEntry>,
	pub kinds: Vec<KindEntry>,
}

/// What part of a team a bite is about (#3049).
///
/// Its offense, defense, special teams, or its coaching (a call: go for it,
/// the try, the clock). Whose is the team's code beside it, never ours or
/// theirs: a game no side is wanted in still has a defense.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Unit {
	Offense,
	Defense,
	SpecialTeams,
	Coaching,
}

impl Unit {
	pub const ALL: [Self; 4] = [Self::Offense, Self::Defense, Self::SpecialTeams, Self::Coaching];

	/// The stored name.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Offense => "offense",
			Self::Defense => "defense",
			Self::SpecialTeams => "special-teams",
			Self::Coaching => "coaching",
		}
	}

	/// The unit stored as `name`.
	#[must_use]
	pub fn named(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|unit| unit.as_str() == name)
	}
}

/// What a claim measures (#3041, D26).
///
/// Each is a figure per team, over the season's games, that home can compute
/// from the plays it keeps: the down, distance, spot, type and score of each,
/// and what the scenario engine already reads of them (timeouts, turnovers,
/// big plays). The weekly verdict pass that computes them is #3041's server
/// half, still to come. Version 3 holds eight of the nine the owner's
/// prototype lists (section 2c): challenges won needs a reader of ESPN's
/// challenge text, and home's recorded replay holds one challenge to prove
/// one on, so it waits for a later version.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Measure {
	/// Sacks per opponent dropback (a pass play or a sack).
	PassRush,
	/// The share of third downs converted: a first down or a score.
	ThirdDown,
	/// Touchdowns per red-zone trip: a possession that snaps 20 yards or
	/// fewer from the end zone.
	RedZone,
	/// The share of fourth downs outside the opponent's 40 the offense goes
	/// for rather than punts or kicks.
	FourthDownGo,
	/// Points per two-minute drive: a possession that starts in a half's last
	/// two minutes.
	TwoMinute,
	/// Timeouts left at the end of the first half.
	TimeoutsAtHalf,
	/// Turnovers forced: interceptions and fumbles recovered.
	Takeaways,
	/// Big plays allowed, at the big-play algebra's smallest size.
	ExplosivesAllowed,
}

/// Which way a measure is good (#3041). `None` for a style, neither good nor
/// bad: going for it on fourth down.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Better {
	Higher,
	Lower,
}

impl Measure {
	pub const ALL: [Self; 8] = [
		Self::PassRush,
		Self::ThirdDown,
		Self::RedZone,
		Self::FourthDownGo,
		Self::TwoMinute,
		Self::TimeoutsAtHalf,
		Self::Takeaways,
		Self::ExplosivesAllowed,
	];

	/// The unit of the team the measure is of.
	#[must_use]
	pub const fn unit(self) -> Unit {
		match self {
			Self::PassRush | Self::Takeaways | Self::ExplosivesAllowed => Unit::Defense,
			Self::ThirdDown | Self::RedZone | Self::TwoMinute => Unit::Offense,
			Self::FourthDownGo | Self::TimeoutsAtHalf => Unit::Coaching,
		}
	}

	/// Which way the measure is good, if either.
	#[must_use]
	pub const fn better(self) -> Option<Better> {
		match self {
			Self::FourthDownGo => None,
			Self::ExplosivesAllowed => Some(Better::Lower),
			Self::PassRush | Self::ThirdDown | Self::RedZone | Self::TwoMinute | Self::TimeoutsAtHalf | Self::Takeaways => Some(Better::Higher),
		}
	}
}

/// The form of a claim (#3041): where a team's measure stands.
///
/// Each shape's sides are the direction of the measure's value, never good
/// or bad: [`Measure::better`] says which direction is good, so a claim on a
/// style (fourth-down go rate) reads the same way as any other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Shape {
	/// Among the highest or lowest N of the 32 teams.
	Rank,
	/// Above or below the league's figure.
	VsLeague,
	/// Above or below a named team's.
	VsTeam,
	/// Rising or falling over the games to come.
	Trend,
}

/// A side of a claim's [`Shape`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Side {
	Highest,
	Lowest,
	Above,
	Below,
	Rising,
	Falling,
}

impl Shape {
	pub const ALL: [Self; 4] = [Self::Rank, Self::VsLeague, Self::VsTeam, Self::Trend];

	/// Its two sides, the higher value first.
	#[must_use]
	pub const fn sides(self) -> [Side; 2] {
		match self {
			Self::Rank => [Side::Highest, Side::Lowest],
			Self::VsLeague | Self::VsTeam => [Side::Above, Side::Below],
			Self::Trend => [Side::Rising, Side::Falling],
		}
	}
}

/// When a claim is settled (#3041).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Deadline {
	/// After N more of the team's games.
	AfterGames,
	/// By a named week of the season.
	ByWeek,
	/// At the regular season's end.
	SeasonEnd,
}

impl Deadline {
	pub const ALL: [Self; 3] = [Self::AfterGames, Self::ByWeek, Self::SeasonEnd];
}

/// What home's numbers say of a claim, each week a new verdict (#3041).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
	/// Not settled, and the numbers so far agree.
	Holding,
	/// Not settled, and the numbers so far disagree.
	AgainstSoFar,
	/// Not settled, and too few games are in to say.
	TooEarly,
	/// Settled, and the owner was right.
	SettledRight,
	/// Settled, and the owner was wrong.
	SettledWrong,
	/// Its bite was deleted: the claim is withdrawn, never lost (the owner,
	/// Nov 18).
	Withdrawn,
}

impl Verdict {
	pub const ALL: [Self; 6] = [Self::Holding, Self::AgainstSoFar, Self::TooEarly, Self::SettledRight, Self::SettledWrong, Self::Withdrawn];
}

/// A measure, as the dump writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct MeasureEntry {
	pub id: Measure,
	pub unit: Unit,
	pub better: Option<Better>,
}

/// A shape, as the dump writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ShapeEntry {
	pub id: Shape,
	pub sides: [Side; 2],
}

/// The claim algebra's closed parts, as the dump lists them.
///
/// Version 3, #3041, D24–D26. A claim's own values (whose, N, the named team
/// or week, and the owner's reason) are the ledger's, not the catalog's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Claims {
	pub measures: Vec<MeasureEntry>,
	pub shapes: Vec<ShapeEntry>,
	pub deadlines: Vec<Deadline>,
	pub verdicts: Vec<Verdict>,
}

/// The whole set, as `dump-game-scenarios` writes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Catalog {
	pub version: u32,
	pub scenarios: Vec<Entry>,
	pub halves: Vec<Half>,
	pub big_play: BigPlays,
	/// What a bite can be about (#3049).
	pub units: Vec<Unit>,
	/// The claim algebra (version 3, #3041).
	pub claims: Claims,
}

/// The set at [`CATALOG_VERSION`].
#[must_use]
pub fn catalog() -> Catalog {
	Catalog {
		version: CATALOG_VERSION,
		scenarios: Scenario::ALL.into_iter().map(Scenario::entry).collect(),
		halves: Half::ALL.to_vec(),
		big_play: BigPlays {
			sizes: Size::ALL.into_iter().map(|id| SizeEntry { id, yards: id.yards() }).collect(),
			kinds: Kind::ALL.into_iter().map(|id| KindEntry { id, least: id.least() }).collect(),
		},
		units: Unit::ALL.to_vec(),
		claims: Claims {
			measures: Measure::ALL
				.into_iter()
				.map(|id| MeasureEntry {
					id,
					unit: id.unit(),
					better: id.better(),
				})
				.collect(),
			shapes: Shape::ALL.into_iter().map(|id| ShapeEntry { id, sides: id.sides() }).collect(),
			deadlines: Deadline::ALL.to_vec(),
			verdicts: Verdict::ALL.to_vec(),
		},
	}
}

#[cfg(test)]
mod tests;
