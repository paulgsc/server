//! A game's moment log (`moment_log`): every play as the scenarios it was,
//! the family "just happened" is the newest few of.

#[cfg(test)]
mod common;

use chrono::TimeDelta;
use common::{first_seen, game, pacific, plays, recorded_log, status, wall, GAME};
use game_scenarios::Scenario;
use nfl_feed::GameState;
use scenario_engine::{moment_log, offer, Mode, Situation, Watching};
use std::collections::HashMap;

/// After every play, everything "just happened" shows in Full mode is an
/// entry of the log as of the same time, field for field: one rule, read
/// twice.
#[test]
fn just_happened_is_the_newest_of_the_log() {
	let plays = plays();
	let log = recorded_log(&plays);
	let mut shown = 0;
	for (k, play) in plays.iter().enumerate() {
		let next = plays.get(k + 1).map(Situation::before);
		let g = game(Some("CHI"), status(GameState::In, Some(play)), next);
		let now = pacific(first_seen(&log, play) + TimeDelta::seconds(20));
		let offered = offer(&g, &log, now, Mode::Full, Watching::Unknown);
		let entries = moment_log(&g, &log, now.to_utc());
		for happened in &offered.just_happened {
			assert!(entries.iter().any(|e| e.happened == *happened), "{happened:?} is not in the log");
			shown += 1;
		}
		// The newest: the play "just happened" leads with is the log's last.
		if let Some(newest) = offered.just_happened.first() {
			assert_eq!(entries.last().unwrap().happened.play_id, newest.play_id);
		}
	}
	assert!(shown > 100, "only {shown} entries were shown");
}

/// The final game's log: in ESPN's order, each entry where its play was in
/// the game, and a big play with its algebra.
#[test]
fn the_finished_games_log_is_every_play_in_order() {
	let plays = plays();
	let log = recorded_log(&plays);
	let last = plays.last().unwrap();
	let g = game(Some("CHI"), status(GameState::Post, Some(last)), None);
	let entries = moment_log(&g, &log, wall(last) + TimeDelta::hours(1));
	let order: HashMap<&str, usize> = plays.iter().enumerate().map(|(k, p)| (p.id.as_str(), k)).collect();
	let mut previous = 0;
	for entry in &entries {
		let k = order[entry.happened.play_id.as_str()];
		assert!(k >= previous, "{} is out of order", entry.happened.play_id);
		previous = k;
		assert_eq!(entry.period, plays[k].period);
		assert_eq!(entry.clock, plays[k].clock);
		assert_eq!(entry.happened.big_play.is_some(), entry.happened.scenario == Scenario::BigPlay);
	}
	// Every kind of moment a whole game has, and more than a live strip holds.
	for scenario in [Scenario::BigPlay, Scenario::Turnover, Scenario::Penalty, Scenario::ThirdAndLong, Scenario::FourthDown] {
		assert!(entries.iter().any(|e| e.happened.scenario == scenario), "no {} in the log", scenario.id());
	}
	let json = serde_json::to_value(&entries[0]).unwrap();
	for field in ["id", "play_id", "line", "period", "clock", "unit", "unit_side"] {
		assert!(json.get(field).is_some(), "an entry has no {field}: {json}");
	}
	assert_eq!(GAME, g.id);
}

/// SE2: replayed at a time, the log holds no play home had not seen by then.
#[test]
fn the_log_at_a_time_holds_only_what_home_had_seen() {
	let plays = plays();
	let log = recorded_log(&plays);
	let half = &plays[plays.len() / 2];
	let g = game(Some("CHI"), status(GameState::In, Some(half)), None);
	let now = first_seen(&log, half);
	let entries = moment_log(&g, &log, now);
	assert!(!entries.is_empty());
	for entry in &entries {
		let play = plays.iter().find(|p| p.id == entry.happened.play_id).unwrap();
		assert!(first_seen(&log, play) <= now);
	}
	// Live, each entry carries the unit read off its play.
	assert!(entries.iter().any(|e| e.happened.unit.is_some()));
}
