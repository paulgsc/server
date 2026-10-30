//! The moment rules one at a time, on the recorded games and on plays shaped
//! from them.

#[cfg(test)]
mod common;

use chrono::{SecondsFormat, TimeDelta};
use common::{at, first_seen, game, is_snap, pacific, plays, recorded_log, status, wall, GAME};
use game_scenarios::{Scenario, CATALOG_VERSION};
use nfl_feed::{
	log::{Entry, PlayLog},
	scoreboard,
	summary::{self, Play},
	GameState,
};
use scenario_engine::{offer, Game, Mode, Offer, Reading, Situation, Watching, ENGINE_VERSION};

const WEEK8: &[u8] = include_bytes!("../../nfl_feed/testdata/scoreboard-2026-week8.json");
const PRE: &[u8] = include_bytes!("../../nfl_feed/testdata/summary-401873023-pre.json");

#[cfg(test)]
fn ids(offer: &Offer) -> Vec<&'static str> {
	offer.scenarios.iter().map(|o| o.scenario.id()).collect()
}

#[cfg(test)]
fn line(offer: &Offer, scenario: Scenario) -> &str {
	&offer.scenarios.iter().find(|o| o.scenario == scenario).unwrap().line
}

/// CAR at GB on 2026-10-29, as the week-8 scoreboard and its summary had it
/// the day before: kickoff 00:15Z (5:15 PM Pacific), 52° and mostly cloudy.
#[cfg(test)]
fn car_at_gb(want: Option<&str>) -> Game {
	let board = scoreboard::parse(WEEK8).unwrap();
	let event = board.games.iter().find(|g| g.id == "401873023").unwrap();
	let summary = summary::parse(PRE).unwrap();
	Game {
		id: event.id.clone(),
		home: event.home.team.clone().unwrap(),
		away: event.away.team.clone().unwrap(),
		kickoff: event.kickoff.unwrap(),
		status: summary.status,
		situation: None,
		weather: event.weather.clone(),
		reading: Reading {
			team: "SF".to_owned(),
			want: want.map(str::to_owned),
		},
	}
}

#[test]
fn pregame_opens_by_the_modes_window_and_lasts_until_kickoff() {
	let g = car_at_gb(Some("GB"));
	let kickoff = g.kickoff;
	let offered = |minutes_before: i64, mode| offer(&g, &[], pacific(kickoff - TimeDelta::minutes(minutes_before)), mode, Watching::Unknown);
	// Two hours out: Standard and Full are open, Light is not.
	assert!(offered(120, Mode::Light).scenarios.is_empty());
	assert_eq!(ids(&offered(120, Mode::Standard)), ["pre.stakes", "pre.weather", "pre.injury-report", "pre.venue"]);
	assert_eq!(offered(120, Mode::Standard).strip.clock, "Kickoff in 2 h");
	assert_eq!(ids(&offered(50, Mode::Light)), ["pre.stakes", "pre.weather"], "Light shows two cards");
	// 8:30 AM Pacific on game day: Full only.
	assert!(offered(525, Mode::Standard).scenarios.is_empty());
	assert_eq!(offered(525, Mode::Full).scenarios.len(), 4);
	assert!(offered(9 * 60 + 20, Mode::Full).scenarios.is_empty(), "7:55 AM is before Full's window");
	// The last quarter hour: the toss leads.
	let close = offered(10, Mode::Full);
	assert_eq!(close.scenarios[0].scenario, Scenario::CoinToss);
	assert_eq!(close.scenarios[0].line, "CAR at GB, kickoff in 10 min");
	assert_eq!(line(&close, Scenario::Weather), "52°, mostly cloudy");
	assert_eq!(line(&close, Scenario::Stakes), "SF wants GB to win");
	assert_eq!(line(&close, Scenario::Venue), "GB at home");
}

#[test]
fn a_game_that_cannot_move_sf_offers_nothing_that_needs_a_side() {
	let g = car_at_gb(None);
	let offered = offer(&g, &[], pacific(g.kickoff - TimeDelta::hours(1)), Mode::Full, Watching::Unknown);
	assert_eq!(ids(&offered), ["pre.weather", "pre.injury-report", "pre.venue"]);
	assert_eq!(line(&offered, Scenario::Venue), "CAR at GB");

	let plays = plays();
	let log = recorded_log(&plays);
	let end = wall(plays.last().unwrap()) + TimeDelta::minutes(5);
	let over = offer(&game(None, status(GameState::Post, None), None), &log, pacific(end), Mode::Full, Watching::Watched);
	assert_eq!(ids(&over), ["post.result", "post.pattern"], "no paths and no you.* without a side");
	assert_eq!(over.scenarios[0].line, "CHI won 30-24");
}

#[test]
fn after_the_final_the_watching_flag_says_which_you_it_was() {
	let plays = plays();
	let log = recorded_log(&plays);
	let end = pacific(wall(plays.last().unwrap()) + TimeDelta::minutes(5));
	let you = |want, watching| {
		let offered = offer(&game(Some(want), status(GameState::Post, None), None), &log, end, Mode::Full, watching);
		offered
			.scenarios
			.into_iter()
			.filter(|o| o.scenario.id().starts_with("you."))
			.map(|o| (o.scenario, o.line))
			.collect::<Vec<_>>()
	};
	assert_eq!(you("CHI", Watching::Watched), [(Scenario::CaughtIt, "You watched CHI win 30-24".to_owned())]);
	assert_eq!(you("CHI", Watching::Skipped), [(Scenario::MissedIt, "You skipped it, and CHI won 30-24".to_owned())]);
	assert_eq!(you("NE", Watching::Watched), [(Scenario::SatThrough, "You watched NE lose 24-30".to_owned())]);
	assert_eq!(you("NE", Watching::Skipped), [(Scenario::DodgedIt, "You skipped it, and NE lost 24-30".to_owned())]);
	assert!(you("NE", Watching::Unknown).is_empty());
	// Tuned in at halftime, NE down 6.
	let late = you("NE", Watching::TunedIn(at("2026-10-23T01:50:00Z")));
	assert_eq!(late[0], (Scenario::TunedInLate, "You tuned in with NE down 6, and watched NE lose 24-30".to_owned()));
	assert_eq!(late[1].0, Scenario::SatThrough);
	let result = offer(&game(Some("NE"), status(GameState::Post, None), None), &log, end, Mode::Full, Watching::Unknown);
	assert_eq!(line(&result, Scenario::Result), "CHI won 30-24, not as SF wanted");
	assert_eq!(line(&result, Scenario::Paths), "What NE's loss does to SF's paths");
}

#[test]
fn a_score_that_puts_the_side_we_want_behind_is_fell_behind() {
	let plays = plays();
	let log = recorded_log(&plays);
	// CHI's first field goal, 0-3: NE (the side SF wants here) fell behind.
	let kick = plays.iter().position(|p| p.id == "401873010344").unwrap();
	let next = plays[kick + 1..].iter().find(|p| is_snap(p)).unwrap();
	let now = pacific(first_seen(&log, &plays[kick]) + TimeDelta::seconds(20));
	let g = game(Some("NE"), status(GameState::In, Some(next)), Some(Situation::before(next)));
	let offered = offer(&g, &log, now, Mode::Full, Watching::Unknown);
	assert_eq!(offered.scenarios[0].scenario, Scenario::FellBehind);
	assert_eq!(offered.scenarios[0].line, "Down 3 after the CHI field goal");
	assert_eq!(offered.just_happened[0].scenario, Scenario::FellBehind);
	// Read through CHI, the same field goal is no swing.
	let offered = offer(&game(Some("CHI"), g.status, g.situation), &log, now, Mode::Full, Watching::Unknown);
	assert!(!ids(&offered).contains(&"in.fell-behind"));
	assert_eq!(offered.just_happened[0].scenario, Scenario::Kick);
}

/// A touchdown as ESPN first lists it, before the try is added to its text.
#[cfg(test)]
fn td_before_the_try(play: &Play) -> Play {
	let text = play.text.as_deref().unwrap();
	let cut = text.find("TOUCHDOWN").unwrap() + "TOUCHDOWN".len();
	Play {
		text: Some(text[..cut].to_owned() + "."),
		home_score: play.home_score.map(|points| points - 1),
		..play.clone()
	}
}

#[test]
fn after_a_touchdown_before_the_try_two_or_one_leads() {
	let plays = plays();
	let td = plays.iter().position(|p| p.id == "401873010832").unwrap();
	// ESPN lists the touchdown, then revises it with the extra point 40 s later.
	let mut log = PlayLog::default();
	let stamp = |t: chrono::DateTime<chrono::Utc>| t.to_rfc3339_opts(SecondsFormat::Secs, true);
	let seen = wall(&plays[td]) + TimeDelta::seconds(10);
	let mut first: Vec<Play> = plays[..=td].to_vec();
	first[td] = td_before_the_try(&plays[td]);
	log.apply(GAME, 1, &stamp(seen), &first);
	log.apply(GAME, 2, &stamp(seen + TimeDelta::seconds(40)), &plays[..=td]);
	let entries: Vec<Entry> = log.entries().to_vec();
	let g = game(Some("NE"), status(GameState::In, Some(&plays[td])), None);

	let pending = offer(&g, &entries, pacific(seen + TimeDelta::seconds(5)), Mode::Standard, Watching::Unknown);
	assert_eq!(ids(&pending)[..2], ["in.two-point", "in.kick"]);
	assert_eq!(pending.scenarios[0].line, "CHI touchdown, up 9 before the try");
	assert_eq!(pending.scenarios[1].line, "Kick it or go for two, up 9");

	// The revision is in: the try is no longer to come, and the touchdown is
	// what just happened, counted from when home first saw it.
	let later = pacific(seen + TimeDelta::seconds(45));
	let done = offer(&g, &entries, later, Mode::Standard, Watching::Unknown);
	assert!(!ids(&done).contains(&"in.two-point"));
	assert_eq!(done.just_happened[0].scenario, Scenario::TwoPoint);
	assert_eq!(done.just_happened[0].line, "CHI touchdown, kicked the extra point");
	assert_eq!(done.just_happened[0].ago, "55 seconds ago");
	assert_eq!(offer(&g, &entries, later, Mode::Light, Watching::Unknown).just_happened.len(), 1);
	// Light keeps a play 60 s from when home first saw it; Full three minutes.
	let gone = pacific(seen + TimeDelta::seconds(61));
	assert!(offer(&g, &entries, gone, Mode::Light, Watching::Unknown).just_happened.is_empty());
	assert!(!offer(&g, &entries, gone, Mode::Full, Watching::Unknown).just_happened.is_empty());
}

#[test]
fn the_whole_log_at_a_time_offers_what_the_log_held_then() {
	let plays = plays();
	let log = recorded_log(&plays);
	for play in plays.iter().step_by(7) {
		let now = wall(play) + TimeDelta::seconds(25);
		let held: Vec<Entry> = log.iter().filter(|e| at(&e.seen_at) <= now).cloned().collect();
		let g = game(Some("CHI"), status(GameState::In, Some(play)), Some(Situation::before(play)));
		assert_eq!(
			offer(&g, &log, pacific(now), Mode::Full, Watching::Unknown),
			offer(&g, &held, pacific(now), Mode::Full, Watching::Unknown),
			"at {now}"
		);
	}
	// Another game's entries are not this game's.
	let mut other = log;
	for entry in &mut other {
		entry.game_id = "elsewhere".to_owned();
	}
	let g = game(Some("CHI"), status(GameState::In, None), None);
	let now = pacific(wall(&plays[40]));
	assert_eq!(offer(&g, &other, now, Mode::Full, Watching::Unknown).strip.score, Some("No score yet".to_owned()));
}

#[test]
fn the_two_minute_line_counts_the_timeouts_left() {
	let plays = plays();
	let log = recorded_log(&plays);
	// 1:28 left, NE down 9 and out of timeouts.
	let play = plays.iter().find(|p| p.sequence == Some(399_800)).unwrap();
	let now = pacific(wall(play) - TimeDelta::seconds(10));
	let read_for = |want| {
		offer(
			&game(Some(want), status(GameState::In, Some(play)), Some(Situation::before(play))),
			&log,
			now,
			Mode::Standard,
			Watching::Unknown,
		)
	};
	let ne = read_for("NE");
	assert_eq!(line(&ne, Scenario::TwoMinute), "1:28 left, no timeouts");
	assert_eq!(line(&read_for("CHI"), Scenario::TwoMinute), "1:28 left, NE has no timeouts");
	assert_eq!(ne.strip.clock, "1:28 left in the 4th");
	assert_eq!(ne.strip.possession.as_deref(), Some("NE has the ball"));
	assert_eq!(ne.strip.down.as_deref(), Some("1st and 10 at the NE 25"));
	assert_eq!(ne.strip.score.as_deref(), Some("CHI 30, NE 21"));
}

#[test]
fn third_and_long_reads_like_the_board_says() {
	let plays = plays();
	let log = recorded_log(&plays);
	// 3rd and 8 at the NE 36, NE down 3.
	let play = plays.iter().find(|p| p.sequence == Some(66_900)).unwrap();
	let now = pacific(wall(play) - TimeDelta::seconds(10));
	let g = game(Some("NE"), status(GameState::In, Some(play)), Some(Situation::before(play)));
	let offered = offer(&g, &log, now, Mode::Standard, Watching::Unknown);
	assert_eq!(offered.scenarios[0].scenario, Scenario::ThirdAndLong);
	assert_eq!(offered.scenarios[0].line, "3rd and 8, down 3");
	let g = game(Some("CHI"), g.status, g.situation);
	assert_eq!(offer(&g, &log, now, Mode::Standard, Watching::Unknown).scenarios[0].line, "NE, 3rd and 8, down 3");
}

#[test]
fn without_the_live_situation_nothing_that_needs_the_down_is_offered() {
	let plays = plays();
	let log = recorded_log(&plays);
	let play = plays.iter().find(|p| p.sequence == Some(66_900)).unwrap();
	let g = game(Some("NE"), status(GameState::In, Some(play)), None);
	let offered = offer(&g, &log, pacific(wall(play) - TimeDelta::seconds(10)), Mode::Full, Watching::Unknown);
	assert!(offered.scenarios.is_empty(), "{:?}", ids(&offered));
	assert_eq!((offered.strip.possession, offered.strip.down), (None, None));
}

#[test]
fn an_offer_carries_its_versions_and_ids() {
	let plays = plays();
	let log = recorded_log(&plays);
	let play = plays.iter().find(|p| p.sequence == Some(66_900)).unwrap();
	let g = game(Some("NE"), status(GameState::In, Some(play)), Some(Situation::before(play)));
	let offered = offer(&g, &log, pacific(wall(play) + TimeDelta::seconds(50)), Mode::Standard, Watching::Unknown);
	let json = serde_json::to_value(&offered).unwrap();
	assert_eq!(json["engine_version"], ENGINE_VERSION);
	assert_eq!(json["catalog_version"], CATALOG_VERSION);
	assert_eq!(json["game_id"], GAME);
	assert_eq!(json["scenarios"][0]["id"], "in.third-long");
	assert_eq!(json["just_happened"][0]["id"], "in.third-long");
	assert_eq!(json["just_happened"][0]["line"], "3rd and 8: stopped");
	assert!(json["just_happened"][0]["ago_seconds"].as_i64().unwrap() >= 50);
}
