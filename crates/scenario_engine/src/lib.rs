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

use chrono::{DateTime, FixedOffset, NaiveTime, TimeDelta, TimeZone, Utc};
use game_scenarios::{Scenario, CATALOG_VERSION};
use nfl_feed::{log::Entry, summary::Play, Status, Weather};
use serde::{Serialize, Serializer};

/// The engine's own version. Every offer carries it, so a bite records the
/// rules it was offered under; bump it when a rule or a line changes.
pub const ENGINE_VERSION: u32 = 1;

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
/// `situation` (see the README). Without one the engine offers nothing that
/// needs the down.
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
