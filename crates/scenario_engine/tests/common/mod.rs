//! A recorded game, kept the way home keeps it: NE at CHI on 2026-10-22
//! (`401873010`), the finished game GWB2 recorded from ESPN
//! (`crates/nfl_feed/testdata`), fed through the play log's own rule
//! ([`PlayLog`]) as the poller would have fetched it live.

#![allow(dead_code, unreachable_pub)] // each test file uses part of it

use chrono::{DateTime, FixedOffset, SecondsFormat, TimeDelta, Utc};
use nfl_feed::{
	log::{Entry, PlayLog},
	summary::{self, Play},
	GameState, Status,
};
use scenario_engine::{Game, Reading, Situation};

pub const FINAL: &[u8] = include_bytes!("../../../nfl_feed/testdata/summary-401873010-final.json");
pub const GAME: &str = "401873010";

pub fn at(time: &str) -> DateTime<Utc> {
	DateTime::parse_from_rfc3339(time).unwrap().with_timezone(&Utc)
}

/// The owner's offset that October: Pacific daylight time.
pub fn pacific(time: DateTime<Utc>) -> DateTime<FixedOffset> {
	time.with_timezone(&FixedOffset::west_opt(7 * 3600).unwrap())
}

pub fn plays() -> Vec<Play> {
	summary::parse(FINAL).unwrap().plays
}

/// When the play happened, by ESPN.
pub fn wall(play: &Play) -> DateTime<Utc> {
	at(play.wallclock.as_deref().unwrap())
}

/// How far ESPN's summary runs behind a play, and how often home fetches it
/// while the game is live (`GAME_FEED_LIVE_SECONDS`).
pub const ESPN_DELAY: TimeDelta = TimeDelta::seconds(5);
pub const CADENCE: TimeDelta = TimeDelta::seconds(15);

/// The log as home would have kept it: a summary every 15 s from a minute
/// before the first play, each holding the plays ESPN had by then.
pub fn recorded_log(plays: &[Play]) -> Vec<Entry> {
	let mut log = PlayLog::default();
	let end = wall(plays.last().unwrap()) + TimeDelta::minutes(1);
	let mut fetched = wall(&plays[0]) - TimeDelta::minutes(1);
	let mut response = 0;
	while fetched <= end {
		let held: Vec<Play> = plays.iter().filter(|p| wall(p) + ESPN_DELAY <= fetched).cloned().collect();
		log.apply(GAME, response, &fetched.to_rfc3339_opts(SecondsFormat::Secs, true), &held);
		fetched += CADENCE;
		response += 1;
	}
	log.entries().to_vec()
}

/// When home first saw `play`.
pub fn first_seen(log: &[Entry], play: &Play) -> DateTime<Utc> {
	log.iter().find(|e| e.play.id == play.id).map(|e| at(&e.seen_at)).unwrap()
}

/// A play from scrimmage, by ESPN's own fields.
pub fn is_snap(play: &Play) -> bool {
	let kind = play.play_type.as_deref().unwrap_or_default();
	play.down.is_some() && play.yards_to_endzone.is_some_and(|y| y > 0) && !["Timeout", "End", "warning", "Kickoff"].iter().any(|word| kind.contains(word))
}

pub fn status(state: GameState, play: Option<&Play>) -> Status {
	Status {
		state: Some(state),
		completed: Some(state == GameState::Post),
		period: play.and_then(|p| p.period),
		clock: play.and_then(|p| p.clock.clone()),
	}
}

/// NE at CHI, read through SF wanting `want` to win.
pub fn game(want: Option<&str>, status: Status, situation: Option<Situation>) -> Game {
	Game {
		id: GAME.to_owned(),
		home: "CHI".to_owned(),
		away: "NE".to_owned(),
		kickoff: at("2026-10-23T00:15:00Z"),
		status,
		situation,
		weather: None,
		reading: Reading {
			team: "SF".to_owned(),
			want: want.map(str::to_owned),
		},
	}
}
