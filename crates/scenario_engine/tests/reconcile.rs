//! What the reconciler (GWB7) reads from the engine, on the recorded game:
//! the plays a scenario's rule says were one, and every scenario that could
//! have been happening at a time. Both are the rules the offer uses (SE3), so
//! they are tested against the offer itself.

mod common;

use chrono::TimeDelta;
use common::{first_seen, game, pacific, plays, recorded_log, status, wall};
use game_scenarios::Scenario;
use nfl_feed::GameState;
use scenario_engine::{about_a_play, could_be, offer, plays_that_were, Mode, Watching};

#[test]
fn every_play_just_happened_is_one_its_scenario_was() {
	let plays = plays();
	let log = recorded_log(&plays);
	let mut checked = 0;
	for play in &plays {
		let now = first_seen(&log, play) + TimeDelta::seconds(20);
		let g = game(Some("CHI"), status(GameState::In, Some(play)), None);
		for happened in offer(&g, &log, pacific(now), Mode::Full, Watching::Unknown).just_happened {
			let were = plays_that_were(&g, &log, now, happened.scenario);
			assert!(
				were.iter().any(|was| was.play_id == happened.play_id),
				"{} was {} at {now}, but plays_that_were leaves it out",
				happened.play_id,
				happened.scenario.id()
			);
			checked += 1;
		}
	}
	assert!(checked > 50, "only {checked} plays just happened");
}

#[test]
fn a_third_and_long_was_a_play_snapped_on_one() {
	let plays = plays();
	let log = recorded_log(&plays);
	let end = wall(plays.last().unwrap()) + TimeDelta::minutes(2);
	let g = game(Some("CHI"), status(GameState::Post, plays.last()), None);
	let were = plays_that_were(&g, &log, end, Scenario::from_id("in.third-long").unwrap());
	assert!(!were.is_empty());
	for was in &were {
		let play = plays.iter().find(|p| p.id == was.play_id).unwrap();
		assert_eq!(play.down, Some(3));
		assert!(play.distance.unwrap() >= 7);
		assert_eq!(was.at, wall(play));
	}
	// In ESPN's order, which is the order they happened.
	assert!(were.windows(2).all(|w| w[0].at <= w[1].at));
	// Nothing home saw after `now` is read (SE2).
	let early = wall(plays.iter().find(|p| p.id == were[1].play_id).unwrap()) - TimeDelta::seconds(1);
	assert!(plays_that_were(&g, &log, early, Scenario::from_id("in.third-long").unwrap()).len() <= 1);
}

#[test]
fn a_scenario_about_no_play_is_never_a_play() {
	let plays = plays();
	let log = recorded_log(&plays);
	let end = wall(plays.last().unwrap()) + TimeDelta::minutes(2);
	let g = game(Some("CHI"), status(GameState::Post, plays.last()), None);
	let mut about_none = 0;
	for scenario in Scenario::ALL {
		if !about_a_play(scenario) {
			about_none += 1;
			assert!(plays_that_were(&g, &log, end, scenario).is_empty(), "{}", scenario.id());
		}
	}
	assert!(about_none > 0);
	assert!(about_a_play(Scenario::from_id("in.sack").unwrap()));
	assert!(!about_a_play(Scenario::from_id("in.two-minute").unwrap()));
}

#[test]
fn what_could_be_holds_everything_offered() {
	let plays = plays();
	let log = recorded_log(&plays);
	let mut checked = 0;
	for play in plays.iter().step_by(3) {
		for now in [wall(play) - TimeDelta::seconds(10), first_seen(&log, play) + TimeDelta::seconds(20)] {
			let g = game(Some("CHI"), status(GameState::In, Some(play)), Some(scenario_engine::Situation::before(play)));
			let could = could_be(&g, &log, pacific(now), Watching::Unknown);
			let offered = offer(&g, &log, pacific(now), Mode::Full, Watching::Unknown);
			for scenario in offered.scenarios.iter().map(|o| o.scenario).chain(offered.just_happened.iter().map(|h| h.scenario)) {
				assert!(could.contains(&scenario), "{} offered at {now} but not in could_be", scenario.id());
				checked += 1;
			}
		}
	}
	assert!(checked > 100, "only {checked} offered");
}

#[test]
fn what_could_be_is_not_everything() {
	let plays = plays();
	let log = recorded_log(&plays);
	let play = plays.iter().find(|p| p.down == Some(1)).unwrap();
	let now = wall(play) - TimeDelta::seconds(10);
	let g = game(Some("CHI"), status(GameState::In, Some(play)), Some(scenario_engine::Situation::before(play)));
	let could = could_be(&g, &log, pacific(now), Watching::Unknown);
	assert!(!could.contains(&Scenario::from_id("post.result").unwrap()));
	assert!(!could.contains(&Scenario::from_id("pre.weather").unwrap()));
}
