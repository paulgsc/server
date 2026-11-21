//! The scenario engine (Game Week Bites, paulgsc/some-ui#2074, story GWB3):
//! what can be happening now in a game, and what just happened, as scenarios
//! from the fixed set ([`game_scenarios`]).
//!
//! "Scenarios are a function of time": [`offer`] takes a game, its play log
//! ([`nfl_feed::log::Entry`], as `game_feed_repo::log` reads it back), the
//! time, the recorder's [`Mode`] and the owner's [`Watching`] flag, and
//! returns the [`Offer`] the phone shows: the live strip, the scenarios that
//! fit this moment (most specific first), what just happened, and the
//! versions it was made under.
//!
//! What this crate promises, SE1 to SE5 (pure, replayable at any time, one
//! rule per moment, the person's terms, read through one team), is written
//! once, in the README's "Invariants".

mod read;
mod rules;
mod words;

use chrono::{DateTime, FixedOffset, NaiveTime, Offset, TimeDelta, TimeZone, Utc};
use game_scenarios::{Scenario, CATALOG_VERSION};
use nfl_feed::{log::Entry, summary::Play, Status, Weather};
use serde::{Serialize, Serializer};

/// The engine's own version. Every offer carries it, so a bite records the
/// rules it was offered under; bump it when a rule or a line changes.
pub const ENGINE_VERSION: u32 = 3;

/// The recorder's three modes (the epic's "Settings: three modes").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
	Light,
	/// The recommended one.
	Standard,
	Full,
}

impl Mode {
	/// How many scenarios the phone shows as cards.
	#[must_use]
	pub const fn cards(self) -> usize {
		match self {
			Self::Light => 2,
			Self::Standard => 4,
			Self::Full => 6,
		}
	}

	/// How many "just happened" entries it shows.
	#[must_use]
	pub const fn just_happened(self) -> usize {
		match self {
			Self::Light => 1,
			Self::Standard => 2,
			Self::Full => 3,
		}
	}

	/// How long a play stays in "just happened" after home first saw it.
	#[must_use]
	pub const fn kept(self) -> TimeDelta {
		match self {
			Self::Light => TimeDelta::seconds(60),
			Self::Standard => TimeDelta::seconds(90),
			Self::Full => TimeDelta::seconds(180),
		}
	}

	/// When the pregame scenarios open: an hour before kickoff (Light), three
	/// hours (Standard), or 8 AM on game day where the owner is (Full), and
	/// never later than Standard's.
	#[must_use]
	pub fn pregame_opens(self, kickoff: DateTime<Utc>, local: FixedOffset) -> DateTime<Utc> {
		match self {
			Self::Light => kickoff - TimeDelta::hours(1),
			Self::Standard => kickoff - TimeDelta::hours(3),
			Self::Full => {
				let standard = kickoff - TimeDelta::hours(3);
				let game_day = kickoff.with_timezone(&local).date_naive();
				local
					.from_local_datetime(&game_day.and_time(NaiveTime::MIN + TimeDelta::hours(8)))
					.single()
					.map_or(standard, |eight| eight.with_timezone(&Utc).min(standard))
			}
		}
	}
}

/// Whether the owner watched the game, from the phone's flag for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Watching {
	/// The phone has not said.
	Unknown,
	/// Not watched.
	Skipped,
	/// Watched from the start.
	Watched,
	/// Watched from this time on, after kickoff.
	TunedIn(DateTime<Utc>),
}

/// The team every game is read through, and the result it wants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reading {
	/// The team followed (`SF`).
	pub team: String,
	/// The side whose win the team wants: the team itself in its own game,
	/// the side Playoff Delta's pieces want in another (`want` in
	/// `story::NextPiece`), or `None` for a game that cannot move it.
	pub want: Option<String>,
}

/// The next snap, as the live feed says: the down, distance, spot and the
/// team with the ball.
///
/// The play log holds each play as it stood at its own snap, so the snap
/// still to come is not in it; the caller reads it from ESPN's live
/// situation (`game_feed_repo::situation`, converted with `From`; see the
/// README). Without one the engine offers nothing that needs the down.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Situation {
	pub possession: Option<String>,
	pub down: Option<i64>,
	pub distance: Option<i64>,
	/// Yards to the end zone for the team with the ball.
	pub yards_to_endzone: Option<i64>,
}

impl Situation {
	/// The situation at `play`'s snap. A replay, which knows the next play,
	/// passes the one at the next snap.
	#[must_use]
	pub fn before(play: &Play) -> Self {
		Self {
			possession: play.possession.clone(),
			down: play.down,
			distance: play.distance,
			yards_to_endzone: play.yards_to_endzone,
		}
	}
}

/// ESPN's live situation (`nfl_feed`'s), as the engine reads it. The
/// timeouts it also carries are not read: the engine counts them from the
/// log's timeout plays, and preferring ESPN's count would change a rule.
impl From<&nfl_feed::situation::Situation> for Situation {
	fn from(live: &nfl_feed::situation::Situation) -> Self {
		Self {
			possession: live.possession.clone(),
			down: live.down,
			distance: live.distance,
			yards_to_endzone: live.yards_to_endzone,
		}
	}
}

/// One game, as of `now`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Game {
	/// ESPN's event id: the log's `game_id`.
	pub id: String,
	pub home: String,
	pub away: String,
	pub kickoff: DateTime<Utc>,
	/// The status as of `now`, from the latest summary or scoreboard.
	pub status: Status,
	pub situation: Option<Situation>,
	/// The forecast, before kickoff.
	pub weather: Option<Weather>,
	pub reading: Reading,
}

/// What the phone shows for one game at one time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Offer {
	pub engine_version: u32,
	pub catalog_version: u32,
	pub game_id: String,
	pub strip: Strip,
	/// The scenarios that fit the moment, most specific first, at most the
	/// mode's cards. The first one shows large.
	pub scenarios: Vec<Offered>,
	/// Recent plays as scenarios, newest first, at most the mode's count.
	pub just_happened: Vec<Happened>,
}

/// The live strip, in words.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Strip {
	/// `14:53 left in the 1st`, `Kickoff in 2 h 10 min`, `Final`, `Not played`.
	pub clock: String,
	/// `NE has the ball`.
	pub possession: Option<String>,
	/// `3rd and 8 at the CHI 35`.
	pub down: Option<String>,
	/// `CHI 13, NE 7`.
	pub score: Option<String>,
}

/// A scenario on offer, with the line that says why now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Offered {
	#[serde(rename = "id", serialize_with = "scenario_id")]
	pub scenario: Scenario,
	/// `3rd and 8, down 4`.
	pub line: String,
}

/// A play that just happened, as a scenario.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Happened {
	#[serde(rename = "id", serialize_with = "scenario_id")]
	pub scenario: Scenario,
	/// ESPN's play id.
	pub play_id: String,
	/// `NE sacked on 4th and 5`.
	pub line: String,
	/// Seconds since the play, as of `now`.
	pub ago_seconds: i64,
	/// `40 seconds ago`.
	pub ago: String,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde passes the field by reference
fn scenario_id<S: Serializer>(scenario: &Scenario, s: S) -> Result<S::Ok, S::Error> {
	s.serialize_str(scenario.id())
}

/// The offer for `game` at `now`, from its play log `log`.
///
/// `now` carries the owner's UTC offset, which places Full mode's 8 AM.
/// Entries of other games, and entries home saw after `now`, are left out
/// (SE2).
#[must_use]
pub fn offer(game: &Game, log: &[Entry], now: DateTime<FixedOffset>, mode: Mode, watching: Watching) -> Offer {
	let at = now.with_timezone(&Utc);
	let so_far = read::SoFar::new(game, log, at);
	let mut scenarios = rules::now(game, &so_far, at, mode, *now.offset(), watching);
	scenarios.truncate(mode.cards());
	let mut just_happened = rules::just_happened(game, &so_far, at, mode);
	just_happened.truncate(mode.just_happened());
	Offer {
		engine_version: ENGINE_VERSION,
		catalog_version: CATALOG_VERSION,
		game_id: game.id.clone(),
		strip: words::strip(game, &so_far, at),
		scenarios,
		just_happened,
	}
}

/// A play the log held, as one scenario: what the reconciler (GWB7) matches a
/// bite to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Was {
	/// ESPN's play id.
	pub play_id: String,
	/// When it happened: ESPN's wallclock, or else when home first saw it.
	pub at: DateTime<Utc>,
	/// When home first saw it.
	pub first: DateTime<Utc>,
}

/// When a bite was said relative to the play it is about (#3040).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Said {
	/// Before its play, which is the snap to come itself: the bite took an
	/// offer of a third and long, a fourth down, a field goal try, a play
	/// call, or the opening kickoff.
	BeforeTheSnap,
	/// Before its play, which ends the drive the snap to come is in: the bite
	/// took an offer of a team's first possession or a trip inside the 20.
	BeforeTheDrive,
	/// Before the opening kickoff, offered up to a quarter hour ahead of it:
	/// its play is that kickoff, however long after.
	BeforeTheKickoff,
	/// Not tied to an offer (picked from the whole set, or said first and
	/// mapped later), so its play is the nearer in time of the nearest at or
	/// before it and the first after it.
	Either,
	/// After its play: what just happened, or an offer about a play already in
	/// the log (the try is read off its touchdown's own play), so its play is
	/// the nearest at or before it.
	After,
}

/// Whether `scenario` was on offer in `game` at `at`: its moment's rule fits
/// then, in any mode, before a mode's count is applied.
#[must_use]
pub fn on_offer(game: &Game, log: &[Entry], at: DateTime<Utc>, scenario: Scenario) -> bool {
	let so_far = read::SoFar::new(game, log, at);
	rules::now(game, &so_far, at, Mode::Full, Utc.fix(), Watching::Unknown)
		.iter()
		.any(|offered| offered.scenario == scenario)
}

/// When a bite offered as `scenario` at `at` was said relative to its play.
///
/// From `game` and `log` as they stood then (SE2): the moment's rule decides,
/// and one rule decides by the moment (a kick offered while a try is pending
/// is about the touchdown's play, one offered on 4th down about the snap to
/// come).
///
/// A bite chosen any other way is the caller's to place: what just happened
/// is [`Said::After`]; one picked from the whole set, or said first and
/// mapped later, is [`Said::Either`].
#[must_use]
pub fn offered_said(game: &Game, log: &[Entry], at: DateTime<Utc>, scenario: Scenario) -> Said {
	let so_far = read::SoFar::new(game, log, at);
	rules::offered_said(game, &so_far, at, scenario)
}

/// The play a bite of `scenario`, said as `said`, is about, and the gap.
///
/// `game` and `log` are as of `now`, after the game (each play at its latest
/// version, SE2). The bite started at `at`; `offered` is when the offer it
/// took was last on home's screen, which is `at` unless the phone was still
/// showing an offer home had moved on from. The gap is `at` minus the play's
/// time, in whole seconds.
///
/// - [`Said::After`]: the nearest play the scenario's rule accepts at or
///   before `at`, at most `window` before.
/// - Before: the snap to come is the first play from scrimmage, or kickoff,
///   that home had not seen by `offered` and that no flag wiped out, its time
///   within `window` either side of `at` (the broadcast runs behind, so it
///   can have happened before the owner started). [`Said::BeforeTheSnap`]:
///   that snap, if the rule accepts it, else none. [`Said::BeforeTheDrive`]:
///   the first play from it on, in the same team's possession, the rule
///   accepts, however long after (the window bounds only the snap to come).
/// - [`Said::BeforeTheKickoff`]: the opening kickoff, if home had not seen it
///   by `offered`, with no window.
/// - [`Said::Either`]: the nearer in time of the [`Said::After`] play and
///   the first the rule accepts after `at`, within `window`; a tie looks back.
#[must_use]
#[allow(clippy::too_many_arguments)] // the bite's two times, its scenario and how, the game and the window
pub fn its_play(
	game: &Game,
	log: &[Entry],
	now: DateTime<Utc>,
	scenario: Scenario,
	said: Said,
	at: DateTime<Utc>,
	offered: DateTime<Utc>,
	window: TimeDelta,
) -> Option<(Was, i64)> {
	let so_far = read::SoFar::new(game, log, now);
	let i = rules::its_play(game, &so_far, now, scenario, said, at, offered, window)?;
	let seen = &so_far.plays[i];
	Some((
		Was {
			play_id: seen.play.id.clone(),
			at: seen.at,
			first: seen.first,
		},
		(at - seen.at).num_seconds(),
	))
}

/// Whether `scenario` can be a play at all. Pregame, the two-minute drill,
/// the final and the owner's own scenarios are about a stretch of the game,
/// so no play is ever one of them.
#[must_use]
pub const fn about_a_play(scenario: Scenario) -> bool {
	rules::about_a_play(scenario.entry().moment)
}

/// Every play of `game` in `log`, as of `now`, that `scenario`'s rule says
/// was one.
///
/// In ESPN's order. It is the rule "just happened" uses (SE3), with no window and no count, so
/// a bite can be matched to any play of its game. Each play is read at its
/// latest version seen by `now` (SE2): replayed after the final, a play a
/// flag wiped out is not a third and long that was stopped.
#[must_use]
pub fn plays_that_were(game: &Game, log: &[Entry], now: DateTime<Utc>, scenario: Scenario) -> Vec<Was> {
	let so_far = read::SoFar::new(game, log, now);
	rules::were(game, &so_far, now, scenario)
		.into_iter()
		.map(|i| Was {
			play_id: so_far.plays[i].play.id.clone(),
			at: so_far.plays[i].at,
			first: so_far.plays[i].first,
		})
		.collect()
}

/// Every scenario that could have been happening in `game` at `now`.
///
/// In the catalog's order: each one whose rule fits then, and each one a play home
/// first saw within Full mode's window was, with no mode's count applied.
///
/// The reconciler (GWB7) accepts a scenario proposed for a bite said first
/// only if it is here at the bite's time. `now` carries the owner's UTC
/// offset, as in [`offer`].
#[must_use]
pub fn could_be(game: &Game, log: &[Entry], now: DateTime<FixedOffset>, watching: Watching) -> Vec<Scenario> {
	let at = now.with_timezone(&Utc);
	let so_far = read::SoFar::new(game, log, at);
	rules::could_be(game, &so_far, at, *now.offset(), watching)
}
