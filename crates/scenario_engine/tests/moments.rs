//! The moment rules one at a time, on the recorded games and on plays shaped
//! from them.

#[cfg(test)]
mod common;

use chrono::{SecondsFormat, TimeDelta};
use common::{at, first_seen, game, is_snap, pacific, plays, recorded_log, status, wall, GAME};
use game_scenarios::{Kind, Scenario, Size};
use nfl_feed::{
	log::{Entry, PlayLog},
	scoreboard,
	summary::{self, Play},
	GameState, Status,
};
use scenario_engine::{offer, BigPlay, Game, Mode, Offer, Reading, Situation, Watching};

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
	// No kick is claimed before the try is in the log.
	assert!(pending.just_happened.iter().all(|j| !j.line.contains("kicked")));
	assert!(pending.just_happened.iter().any(|j| j.line == "CHI touchdown, the try to come"));
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
fn an_offer_names_each_scenario_by_its_id() {
	let plays = plays();
	let log = recorded_log(&plays);
	let play = plays.iter().find(|p| p.sequence == Some(66_900)).unwrap();
	let g = game(Some("NE"), status(GameState::In, Some(play)), Some(Situation::before(play)));
	let offered = offer(&g, &log, pacific(wall(play) + TimeDelta::seconds(50)), Mode::Standard, Watching::Unknown);
	let json = serde_json::to_value(&offered).unwrap();
	assert_eq!(json["game_id"], GAME);
	assert_eq!(json["scenarios"][0]["id"], "in.third-long");
	assert_eq!(json["just_happened"][0]["id"], "in.third-long");
	assert_eq!(json["just_happened"][0]["line"], "3rd and 8: stopped");
	assert!(json["just_happened"][0]["ago_seconds"].as_i64().unwrap() >= 50);
}

#[test]
fn an_injury_note_names_the_injured_players_team_not_the_offense() {
	let plays = plays();
	let log = recorded_log(&plays);
	let back = plays.iter().find(|p| p.id == "4018730101455").unwrap();
	assert!(back.text.as_deref().unwrap().contains("NE-K.Byard has returned"));
	let now = pacific(first_seen(&log, back) + TimeDelta::seconds(5));
	let held: Vec<Entry> = log.iter().filter(|e| at(&e.seen_at) <= now).cloned().collect();
	let g = game(Some("CHI"), status(GameState::In, Some(back)), None);
	let o = offer(&g, &held, now, Mode::Full, Watching::Unknown);
	let line = &o.just_happened.iter().find(|j| j.scenario == Scenario::Injury).unwrap().line;
	assert_eq!(line, "A NE player is back");
}

/// The offer in Full mode 5 s after home saw `shown` as the latest play,
/// with the recorded plays before it.
#[cfg(test)]
fn just_after(before: &[Play], shown: Play) -> Offer {
	let seen = wall(&shown) + TimeDelta::seconds(10);
	let mut held = before.to_vec();
	held.push(shown);
	let mut log = PlayLog::default();
	log.apply(GAME, 1, &seen.to_rfc3339_opts(SecondsFormat::Secs, true), &held);
	let entries: Vec<Entry> = log.entries().to_vec();
	let g = game(Some("NE"), status(GameState::In, held.last()), None);
	offer(&g, &entries, pacific(seen + TimeDelta::seconds(5)), Mode::Full, Watching::Unknown)
}

#[test]
fn a_turnover_wiped_out_by_a_flag_is_no_turnover() {
	let plays = plays();
	let pick = plays.iter().position(|p| p.text.as_deref().is_some_and(|t| t.contains("INTERCEPTED"))).unwrap();
	let text = plays[pick].text.clone().unwrap();
	let nullified = Play {
		text: Some(text + " PENALTY on NE-C.Gonzalez, Defensive Holding, 5 yards, enforced at CHI 30 - No Play."),
		..plays[pick].clone()
	};
	let o = just_after(&plays[..pick], nullified);
	assert!(o.just_happened.iter().all(|j| j.scenario != Scenario::Turnover));
}

#[test]
fn a_reviewed_play_is_read_as_the_review_left_it() {
	let plays = plays();
	// "... to CHI 1 for 6 yards ... the play was REVERSED.(Shotgun) D.Maye
	// scrambles right guard for 7 yards, TOUCHDOWN.", shaped into a 3rd and 7
	// that the review moved from short to just enough.
	let pick = plays.iter().position(|p| p.sequence == Some(115_000)).unwrap();
	let text = plays[pick].text.clone().unwrap();
	let third = |text: String| Play {
		play_type: Some("Rush".to_owned()),
		down: Some(3),
		distance: Some(7),
		yards_to_endzone: Some(30),
		text: Some(text),
		..plays[pick].clone()
	};
	let third_and_long = |o: &Offer| o.just_happened.iter().find(|j| j.scenario == Scenario::ThirdAndLong).unwrap().line.clone();

	let reversed = text[..text.find(", TOUCHDOWN").unwrap()].to_owned() + ".";
	assert!(reversed.contains("for 6 yards") && reversed.ends_with("REVERSED.(Shotgun) D.Maye scrambles right guard for 7 yards."));
	assert_eq!(third_and_long(&just_after(&plays[..pick], third(reversed))), "3rd and 7: converted");

	// An upheld ruling that is not restated stands as it was.
	let upheld = text[..text.find(".New England").unwrap()].replace("for 6 yards", "for 7 yards")
		+ ". The Replay Official reviewed the ruling, and the play was Upheld. The ruling on the field stands.";
	assert_eq!(third_and_long(&just_after(&plays[..pick], third(upheld))), "3rd and 7: converted");
}

#[test]
fn the_opening_kickoff_is_the_first_kickoff_after_a_coin_toss() {
	let plays = plays();
	let kickoff = plays.iter().position(|p| p.play_type.as_deref() == Some("Kickoff")).unwrap();
	let toss = Play {
		id: "401873010toss".to_owned(),
		sequence: Some(1),
		play_type: Some("Coin Toss".to_owned()),
		text: Some("CHI wins the toss and defers.".to_owned()),
		down: None,
		distance: None,
		yards_to_endzone: None,
		..plays[kickoff].clone()
	};
	let o = just_after(&[toss], plays[kickoff].clone());
	let opening = o.just_happened.iter().find(|j| j.scenario == Scenario::CoinToss).unwrap();
	assert_eq!(opening.play_id, plays[kickoff].id);
	// Once the kickoff is in the log, the coin toss is only in "just happened".
	assert!(!ids(&o).contains(&Scenario::CoinToss.id()), "{:?}", ids(&o));
}

#[test]
fn a_game_called_off_before_it_was_played_offers_nothing() {
	let mut g = car_at_gb(Some("GB"));
	g.status = Status {
		state: Some(GameState::Post),
		completed: Some(false),
		period: None,
		clock: None,
	};
	for at in [g.kickoff - TimeDelta::hours(1), g.kickoff - TimeDelta::minutes(5), g.kickoff + TimeDelta::hours(3)] {
		let o = offer(&g, &[], pacific(at), Mode::Full, Watching::Unknown);
		assert!(o.scenarios.is_empty(), "{:?}", ids(&o));
		assert!(o.just_happened.is_empty());
		assert_eq!(o.strip.clock, "Not played");
	}
}

#[test]
fn a_fourth_down_that_comes_up_short_turns_it_over_on_downs() {
	let plays = plays();
	let short = plays.iter().position(|p| p.id == "401873010224").unwrap();
	assert_eq!(plays[short].down, Some(4));
	let o = just_after(&plays[..short], plays[short].clone());
	let turnover = &o.just_happened.iter().find(|j| j.scenario == Scenario::Turnover).unwrap().line;
	assert_eq!(turnover, "NE turned it over on downs");
}

#[test]
fn a_fourth_down_converted_in_the_red_zone_leaves_the_trip_going() {
	let plays = plays();
	let made = plays.iter().position(|p| p.id == "4018730101368").unwrap();
	assert_eq!(plays[made].down, Some(4));
	let o = just_after(&plays[..made], plays[made].clone());
	assert!(o.just_happened.iter().all(|j| !j.line.ends_with("red zone: empty")));
	assert!(o.just_happened.iter().all(|j| j.scenario != Scenario::Turnover));
}

#[test]
fn a_touchdown_called_back_by_a_flag_is_no_score() {
	let plays = plays();
	let td = plays.iter().position(|p| p.id == "401873010832").unwrap();
	let mut called_back = plays[td].clone();
	let text = td_before_the_try(&plays[td]).text.unwrap();
	called_back.text = Some(text + " PENALTY on CHI-D.Wright, Offensive Holding, 10 yards, enforced at NE 20 - No Play.");
	called_back.home_score = plays[td - 1].home_score;
	called_back.away_score = plays[td - 1].away_score;
	let o = just_after(&plays[..td], called_back);
	assert!(!ids(&o).contains(&"in.two-point"));
	assert!(o.just_happened.iter().all(|j| !j.line.contains("touchdown")));
}

/// FLA vs UGA on 2026-10-31, live (`nfl_feed`'s recorded college summary,
/// the same API's spelling): ESPN's situation reaches the strip and the
/// rules through `Situation::from`.
#[test]
fn espns_live_situation_drives_the_strip() {
	let summary = summary::parse(include_bytes!("../../nfl_feed/testdata/cfb-summary-401856734-live.json")).unwrap();
	let live = summary.situation.as_ref().unwrap();
	let g = Game {
		id: "401856734".to_owned(),
		home: "UGA".to_owned(),
		away: "FLA".to_owned(),
		kickoff: at("2026-10-31T19:30:00Z"),
		status: summary.status.clone(),
		situation: Some(Situation::from(live)),
		weather: None,
		reading: Reading {
			team: "SF".to_owned(),
			want: None,
		},
	};
	let shown = offer(&g, &[], pacific(at("2026-10-31T21:38:00Z")), Mode::Standard, Watching::Unknown);
	assert_eq!(shown.strip.possession.as_deref(), Some("FLA has the ball"));
	assert_eq!(shown.strip.down.as_deref(), Some("2nd and 10 at the FLA 8"));
	assert!(ids(&shown).contains(&"in.play-call"), "{:?}", ids(&shown));
}

/// What the reconciler (GWB7) reads from the engine, on the recorded game:
/// the plays a scenario's rule says were one, and every scenario that could
/// have been happening at a time. Both are the rules the offer uses (SE3), so
/// they are tested against the offer itself.
mod reconcile {
	use crate::common::{first_seen, game, pacific, plays, recorded_log, status, wall};
	use chrono::TimeDelta;
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
}

/// The play's big-play reading, as "just happened" carries it.
#[cfg(test)]
fn big(o: &Offer) -> Option<(&BigPlay, &str)> {
	o.just_happened
		.iter()
		.find(|j| j.scenario == Scenario::BigPlay)
		.map(|j| (j.big_play.as_ref().unwrap(), j.line.as_str()))
}

/// #3048: a big play is credited to the side that gained it. A pick-six is
/// the defense's interception return, never the offense's pass (the text
/// names the target with "intended for", and the return's yards after it).
#[test]
fn a_big_play_is_the_side_that_gained_it() {
	let plays = plays();
	let pick = plays.iter().position(|p| p.text.as_deref().is_some_and(|t| t.contains("INTERCEPTED"))).unwrap();
	assert_eq!(plays[pick].possession.as_deref(), Some("NE"));
	let six = Play {
		play_type: Some("Interception Return Touchdown".to_owned()),
		text: Some("(Shotgun) D.Maye pass short left intended for H.Henry INTERCEPTED by K.Byard at NE 45. K.Byard to NE 0 for 45 yards, TOUCHDOWN.".to_owned()),
		..plays[pick].clone()
	};
	let o = just_after(&plays[..pick], six);
	let (play, line) = big(&o).unwrap();
	assert_eq!(
		(play.kind, play.size, play.by.as_str(), play.yards, play.scored),
		(Kind::InterceptionReturn, Size::Huge, "CHI", 45, true)
	);
	assert_eq!(play.good, Some(false), "read through NE");
	assert_eq!(line, "CHI 45-yard interception return, touchdown");
	// Read through NE, a pick-six is about NE's offense, which threw it.
	let unit = o.ranked.iter().find(|r| r.scenario == Scenario::BigPlay).map(|r| (r.unit, r.unit_side.clone()));
	assert_eq!(unit, Some((Some(game_scenarios::Unit::Offense), Some("NE".to_owned()))));

	// The same play with the return stopped short of 20 is still big: any
	// turnover is, whatever its yards (the owner, C967).
	let short = Play {
		text: Some("(Shotgun) D.Maye pass short left intended for H.Henry INTERCEPTED by K.Byard at NE 45. K.Byard to NE 33 for 12 yards.".to_owned()),
		..plays[pick].clone()
	};
	let o = just_after(&plays[..pick], short);
	let (play, line) = big(&o).unwrap();
	assert_eq!((play.kind, play.size, play.by.as_str(), play.yards), (Kind::InterceptionReturn, Size::Big, "CHI", 12));
	assert_eq!(line, "CHI 12-yard interception return");

	// A punt returned 50 yards is the receivers'.
	let punt = plays
		.iter()
		.position(|p| p.play_type.as_deref() == Some("Punt") && p.possession.as_deref() == Some("NE"))
		.unwrap();
	let returned = Play {
		text: Some("M.Wishnowsky punts 50 yards to CHI 10, Center-J.Ashby. K.Raymond to NE 40 for 50 yards (D.Muasau).".to_owned()),
		..plays[punt].clone()
	};
	let o = just_after(&plays[..punt], returned);
	let (play, line) = big(&o).unwrap();
	assert_eq!((play.kind, play.size, play.by.as_str(), play.good), (Kind::PuntReturn, Size::Huge, "CHI", Some(false)));
	assert_eq!(line, "CHI 50-yard punt return");

	// A pass for 62 and a touchdown is the offense's, and the kind you remember.
	let pass = plays
		.iter()
		.position(|p| p.play_type.as_deref() == Some("Pass Reception") && p.possession.as_deref() == Some("NE"))
		.unwrap();
	let long = Play {
		play_type: Some("Passing Touchdown".to_owned()),
		text: Some("(Shotgun) D.Maye pass deep right to S.Diggs for 62 yards, TOUCHDOWN.".to_owned()),
		..plays[pass].clone()
	};
	let o = just_after(&plays[..pass], long);
	let (play, line) = big(&o).unwrap();
	assert_eq!(
		(play.kind, play.size, play.by.as_str(), play.scored, play.good),
		(Kind::Pass, Size::Memorable, "NE", true, Some(true))
	);
	assert_eq!(line, "NE 62-yard touchdown");
}

/// #3048: a kick return is big from 40 (C871), however ESPN types it.
#[test]
fn a_kick_return_is_big_from_40() {
	let plays = plays();
	// The ordinary kick return, out to about the 25, is not big; 47 is.
	let kickoffs: Vec<usize> = plays
		.iter()
		.enumerate()
		.filter(|(_, p)| p.play_type.as_deref() == Some("Kickoff"))
		.map(|(i, _)| i)
		.collect();
	let ordinary = kickoffs.iter().find(|i| plays[**i].text.as_deref().unwrap().contains("for 26 yards")).unwrap();
	assert!(big(&just_after(&plays[..*ordinary], plays[*ordinary].clone())).is_none());
	let long_return = kickoffs.iter().find(|i| plays[**i].text.as_deref().unwrap().contains("for 47 yards")).unwrap();
	let o = just_after(&plays[..*long_return], plays[*long_return].clone());
	let (play, _) = big(&o).unwrap();
	assert_eq!((play.kind, play.size, play.by.as_str()), (Kind::KickReturn, Size::Huge, "CHI"));

	// ESPN's own type for a kickoff run back all the way.
	let all_the_way = Play {
		play_type: Some("Kickoff Return Touchdown".to_owned()),
		text: Some("C.Santos kicks 65 yards from CHI 35 to NE 0. C.Kiner to CHI 0 for 100 yards, TOUCHDOWN.".to_owned()),
		..plays[*ordinary].clone()
	};
	let o = just_after(&plays[..*ordinary], all_the_way);
	let (play, line) = big(&o).unwrap();
	assert_eq!((play.kind, play.size, play.by.as_str(), play.scored), (Kind::KickReturn, Size::Memorable, "NE", true));
	assert_eq!(line, "NE 100-yard kick return, touchdown");
}

/// #3048: a gain is read as it stood: a fumble the offense fell on keeps its
/// run, and a flag on a return, a muff, or a touchdown taken away are not
/// what the yards say.
#[test]
fn a_big_play_is_read_as_it_stood() {
	let plays = plays();
	let punt = plays
		.iter()
		.position(|p| p.play_type.as_deref() == Some("Punt") && p.possession.as_deref() == Some("NE"))
		.unwrap();
	let pass = plays
		.iter()
		.position(|p| p.play_type.as_deref() == Some("Pass Reception") && p.possession.as_deref() == Some("NE"))
		.unwrap();
	// A run whose fumble the offense fell on is still its run.
	let run = plays
		.iter()
		.position(|p| p.play_type.as_deref() == Some("Rush") && p.possession.as_deref() == Some("NE"))
		.unwrap();
	let own = Play {
		play_type: Some("Fumble Recovery (Own)".to_owned()),
		text: Some("R.Stevenson left end to CHI 30 for 35 yards. FUMBLES, RECOVERED by NE-R.Stevenson at CHI 30.".to_owned()),
		..plays[run].clone()
	};
	let o = just_after(&plays[..run], own);
	let (play, _) = big(&o).unwrap();
	assert_eq!((play.kind, play.yards, play.by.as_str()), (Kind::Run, 35, "NE"));

	// A flag on a return, a muffed punt, and a touchdown taken away are not
	// what the yards say.
	let flagged = Play {
		text: Some("M.Wishnowsky punts 50 yards to CHI 10, Center-J.Ashby. K.Raymond to NE 20 for 70 yards. PENALTY on CHI-J.Brisker, Illegal Block Above the Waist, 10 yards, enforced at CHI 35.".to_owned()),
		..plays[punt].clone()
	};
	assert!(big(&just_after(&plays[..punt], flagged)).is_none());
	let muffed = Play {
		text: Some("M.Wishnowsky punts 50 yards to CHI 10, Center-J.Ashby. K.Raymond MUFFS catch, RECOVERED by NE-D.Muasau at CHI 30. D.Muasau to CHI 5 for 25 yards.".to_owned()),
		..plays[punt].clone()
	};
	// The muff is no punt return, but the kickers got the ball: a turnover,
	// so big whatever its yards (C967).
	let o = just_after(&plays[..punt], muffed);
	let (play, line) = big(&o).unwrap();
	assert_eq!((play.kind, play.size, play.by.as_str()), (Kind::FumbleReturn, Size::Big, "NE"));
	assert_eq!(line, "NE fumble return");
	let taken_away = Play {
		text: Some(
			"(Shotgun) D.Maye pass deep right to S.Diggs for 62 yards, TOUCHDOWN NULLIFIED by Penalty. PENALTY on NE-M.Onwenu, Offensive Holding, 10 yards, enforced at NE 38."
				.to_owned(),
		),
		..plays[pass].clone()
	};
	assert!(big(&just_after(&plays[..pass], taken_away)).is_none(), "a gain the flag took away is none");
	// Every flag is read, and a declined one is none: an accepted block in
	// the back on the returners, beside a declined face mask on the kickers.
	let two_flags = Play {
		text: Some("M.Wishnowsky punts 50 yards to CHI 10, Center-J.Ashby. K.Raymond to NE 40 for 50 yards. PENALTY on NE-D.Muasau, Face Mask, declined. PENALTY on CHI-J.Brisker, Illegal Block Above the Waist, 10 yards, enforced at CHI 35.".to_owned()),
		..plays[punt].clone()
	};
	assert!(big(&just_after(&plays[..punt], two_flags)).is_none());
	let kickers_flag = Play {
		text: Some("M.Wishnowsky punts 50 yards to CHI 10, Center-J.Ashby. K.Raymond to NE 40 for 50 yards. PENALTY on CHI-J.Brisker, Illegal Block Above the Waist, declined. PENALTY on NE-D.Muasau, Face Mask, 15 yards, enforced at NE 40.".to_owned()),
		..plays[punt].clone()
	};
	let o = just_after(&plays[..punt], kickers_flag);
	assert_eq!(
		big(&o).map(|(play, _)| (play.kind, play.yards)),
		Some((Kind::PuntReturn, 50)),
		"a flag on the kickers leaves the return"
	);
	// A return a review reversed is not what its first yards say.
	let pick = plays.iter().position(|p| p.text.as_deref().is_some_and(|t| t.contains("INTERCEPTED"))).unwrap();
	let reversed = Play {
		text: Some("D.Maye pass short left intended for H.Henry INTERCEPTED by K.Byard at NE 45. K.Byard to NE 20 for 25 yards. The Replay Official reviewed the pass completion ruling, and the play was REVERSED. D.Maye pass short left to H.Henry to NE 50 for 5 yards.".to_owned()),
		..plays[pick].clone()
	};
	assert!(big(&just_after(&plays[..pick], reversed)).is_none_or(|(play, _)| play.kind != Kind::InterceptionReturn));
}

/// #3048 (#2908): an onside kick is in "just happened", with who got it back:
/// ESPN's "recovered by" when it was loose, else the receivers, who fielded it.
#[test]
fn an_onside_kick_says_who_recovered_it() {
	let plays = plays();
	let onside = plays.iter().position(|p| p.text.as_deref().is_some_and(|t| t.contains("onside"))).unwrap();
	let line = |o: &Offer| o.just_happened.iter().find(|j| j.scenario == Scenario::OnsideKick).map(|j| j.line.clone());
	assert_eq!(
		line(&just_after(&plays[..onside], plays[onside].clone())).as_deref(),
		Some("NE onside kick, recovered by CHI")
	);
	let loose = Play {
		text: Some("M.Wishnowsky kicks onside 11 yards from NE 35 to NE 46. RECOVERED by NE-B.Bolden.".to_owned()),
		..plays[onside].clone()
	};
	assert_eq!(line(&just_after(&plays[..onside], loose)).as_deref(), Some("NE onside kick, recovered by NE"));
	assert!(
		just_after(&plays[..onside], plays[onside].clone())
			.just_happened
			.iter()
			.all(|j| j.scenario != Scenario::BigPlay),
		"an onside kick is no kick return"
	);
}

/// #3048: halftime is on offer from ESPN's end of the half until the second
/// half's kickoff, with where the half left the side wanted.
#[test]
fn halftime_is_on_offer_until_the_second_half_kicks_off() {
	let plays = plays();
	let half = plays.iter().position(|p| p.play_type.as_deref() == Some("End of Half")).unwrap();
	let at_half = |upto: usize, want: Option<&str>| {
		let seen = wall(&plays[upto]) + TimeDelta::seconds(10);
		let mut log = PlayLog::default();
		log.apply(GAME, 1, &seen.to_rfc3339_opts(SecondsFormat::Secs, true), &plays[..=upto]);
		let entries: Vec<Entry> = log.entries().to_vec();
		let g = game(want, status(GameState::In, Some(&plays[upto])), None);
		offer(&g, &entries, pacific(seen + TimeDelta::seconds(60)), Mode::Standard, Watching::Unknown)
	};
	let o = at_half(half, Some("NE"));
	assert_eq!(ids(&o).first(), Some(&"in.halftime"));
	assert_eq!(line(&o, Scenario::Halftime), "Halftime, NE down 6");
	assert_eq!(line(&at_half(half, None), Scenario::Halftime), "Halftime, CHI 20, NE 14");
	let kickoff = plays[half..].iter().position(|p| p.play_type.as_deref() == Some("Kickoff")).unwrap() + half;
	assert!(!ids(&at_half(kickoff, Some("NE"))).contains(&"in.halftime"), "the second half is under way");
	assert!(!ids(&at_half(half - 1, Some("NE"))).contains(&"in.halftime"), "not before the half ends");
}

/// #3049: the ranked list holds every scenario once: the cards exactly, then
/// the always plausible, then the rest; and each says what a bite of it is
/// about by default, as a unit and the team's code.
#[test]
fn every_scenario_is_ranked_once_with_what_it_is_about() {
	use game_scenarios::Unit;
	use scenario_engine::Tier;
	let plays = plays();
	let log = recorded_log(&plays);
	// NE on 4th down.
	let fourth = plays.iter().position(|p| is_snap(p) && p.down == Some(4) && p.possession.as_deref() == Some("NE")).unwrap();
	let at = pacific(wall(&plays[fourth]) - TimeDelta::seconds(10));
	let at_fourth = |want: Option<&str>, mode| {
		let g = game(want, status(GameState::In, Some(&plays[fourth])), Some(Situation::before(&plays[fourth])));
		offer(&g, &log, at, mode, Watching::Unknown)
	};
	for mode in [Mode::Light, Mode::Standard, Mode::Full] {
		let o = at_fourth(Some("CHI"), mode);
		assert_eq!(o.ranked.len(), Scenario::ALL.len());
		for scenario in Scenario::ALL {
			assert_eq!(o.ranked.iter().filter(|r| r.scenario == scenario).count(), 1, "{scenario:?} once");
		}
		let now: Vec<Scenario> = o.ranked.iter().filter(|r| r.tier == Tier::Now).map(|r| r.scenario).collect();
		assert_eq!(now, o.scenarios.iter().map(|s| s.scenario).collect::<Vec<_>>(), "now is the cards, in their order");
		let tiers: Vec<Tier> = o.ranked.iter().map(|r| r.tier).collect();
		assert!(tiers.windows(2).all(|w| (w[0] as u8) <= (w[1] as u8)), "now, then always, then the rest");
	}
	let about = |o: &Offer, scenario: Scenario| o.ranked.iter().find(|r| r.scenario == scenario).map(|r| (r.unit, r.unit_side.clone())).unwrap();
	let team = |unit, side: &str| (Some(unit), Some(side.to_owned()));
	let o = at_fourth(Some("CHI"), Mode::Full);
	assert_eq!(about(&o, Scenario::FourthDown), team(Unit::Coaching, "NE"), "the call is NE's");
	assert_eq!(
		about(&o, Scenario::PlayCall),
		team(Unit::Defense, "CHI"),
		"NE has the ball: CHI's defense, as the owner says it"
	);
	assert_eq!(about(&o, Scenario::Kick), team(Unit::SpecialTeams, "NE"), "the team kicking");
	assert_eq!(about(&o, Scenario::Weather), (None, None), "pregame is about no unit");
	assert_eq!(about(&o, Scenario::CaughtIt), (None, None), "nor is the owner's own watching");
	assert_eq!(o.ranked.iter().find(|r| r.scenario == Scenario::Stakes).unwrap().tier, Tier::Always);
	// No side wanted: still a defense to talk about, named by the team.
	let o = at_fourth(None, Mode::Full);
	assert_eq!(about(&o, Scenario::FourthDown), team(Unit::Coaching, "NE"));
	assert_eq!(about(&o, Scenario::PlayCall), team(Unit::Offense, "NE"), "the team with the ball's offense");
	assert_eq!(about(&o, Scenario::Halftime), (Some(Unit::Coaching), None), "halftime is no one team's");
}

/// #3049: the defaults the moment cannot read off the snap to come: the
/// try's kick is the scorer's, a lead change is the scorer's even snaps
/// later (while the latest play is the other team's), an onside kick is the kicking team's, and nothing is about a unit
/// once the game is final.
#[test]
fn a_bites_default_unit_follows_who_did_it() {
	use game_scenarios::Unit;
	let plays = plays();
	let about = |o: &Offer, scenario: Scenario| o.ranked.iter().find(|r| r.scenario == scenario).map(|r| (r.unit, r.unit_side.clone())).unwrap();
	let team = |unit, side: &str| (Some(unit), Some(side.to_owned()));

	// CHI's touchdown, its try to come, read through NE.
	let td = plays.iter().position(|p| p.id == "401873010832").unwrap();
	let mut log = PlayLog::default();
	let seen = wall(&plays[td]) + TimeDelta::seconds(10);
	let mut first: Vec<Play> = plays[..=td].to_vec();
	first[td] = td_before_the_try(&plays[td]);
	log.apply(GAME, 1, &seen.to_rfc3339_opts(SecondsFormat::Secs, true), &first);
	let entries: Vec<Entry> = log.entries().to_vec();
	let g = game(Some("NE"), status(GameState::In, Some(&plays[td])), None);
	let o = offer(&g, &entries, pacific(seen + TimeDelta::seconds(5)), Mode::Full, Watching::Unknown);
	assert_eq!(about(&o, Scenario::Kick), team(Unit::SpecialTeams, "CHI"), "the scorer kicks the try");
	assert_eq!(about(&o, Scenario::TwoPoint), team(Unit::Coaching, "CHI"), "and makes the call");

	// NE's second snap after it, so the latest play is NE's own: the score
	// was still CHI's offense, which NE's side calls its defense.
	let later = (td + 1..plays.len())
		.filter(|&i| is_snap(&plays[i]) && plays[i].possession.as_deref() == Some("NE"))
		.nth(1)
		.unwrap();
	let g = game(Some("NE"), status(GameState::In, Some(&plays[later])), Some(Situation::before(&plays[later])));
	let o = offer(
		&g,
		&recorded_log(&plays),
		pacific(wall(&plays[later]) - TimeDelta::seconds(10)),
		Mode::Full,
		Watching::Unknown,
	);
	assert_eq!(about(&o, Scenario::FellBehind), team(Unit::Defense, "NE"), "the scorer's, not the latest play's");

	// The onside kick, NE's.
	let onside = plays.iter().position(|p| p.text.as_deref().is_some_and(|t| t.contains("onside"))).unwrap();
	let o = just_after(&plays[..onside], plays[onside].clone());
	assert_eq!(about(&o, Scenario::OnsideKick), team(Unit::SpecialTeams, "NE"));

	// After the final: about the game, never a unit.
	let end = wall(plays.last().unwrap()) + TimeDelta::minutes(5);
	let g = game(Some("NE"), status(GameState::Post, plays.last()), None);
	let o = offer(&g, &recorded_log(&plays), pacific(end), Mode::Full, Watching::Watched);
	assert!(o.ranked.iter().all(|r| r.unit.is_none() && r.unit_side.is_none()));
}

/// #3049: a play that just happened carries its own default, read off that
/// play: NE's punt on 4th down is NE's call while the 4th down to come, on
/// the list, is CHI's.
#[test]
fn a_play_just_happened_is_about_who_made_it() {
	use game_scenarios::Unit;
	let plays = plays();
	let punt = (0..plays.len() - 1)
		.find(|&i| {
			let p = &plays[i];
			is_snap(p) && p.down == Some(4) && p.play_type.as_deref() == Some("Punt") && p.possession.as_deref() == Some("NE")
		})
		.unwrap();
	let next = plays[punt + 1..].iter().position(is_snap).unwrap() + punt + 1;
	assert_eq!(plays[next].possession.as_deref(), Some("CHI"));
	let g = game(Some("NE"), status(GameState::In, Some(&plays[next])), Some(Situation::before(&plays[next])));
	let o = offer(
		&g,
		&recorded_log(&plays),
		pacific(wall(&plays[next]) - TimeDelta::seconds(10)),
		Mode::Full,
		Watching::Unknown,
	);
	let happened = o.just_happened.iter().find(|j| j.scenario == Scenario::FourthDown && j.play_id == plays[punt].id).unwrap();
	assert_eq!((happened.unit, happened.unit_side.as_deref()), (Some(Unit::Coaching), Some("NE")), "NE punted");
	let ranked = o.ranked.iter().find(|r| r.scenario == Scenario::FourthDown).unwrap();
	assert_eq!((ranked.unit, ranked.unit_side.as_deref()), (Some(Unit::Coaching), Some("CHI")), "CHI has the next 4th down");
}

/// #3049: ESPN says the game is on before the log holds its opening
/// kickoff; until it does, nothing is about a unit.
#[test]
fn nothing_is_about_a_unit_before_the_opening_kickoff() {
	let plays = plays();
	let g = game(Some("NE"), status(GameState::In, None), None);
	let o = offer(&g, &[], pacific(wall(&plays[0]) - TimeDelta::seconds(30)), Mode::Full, Watching::Unknown);
	assert!(o.ranked.iter().all(|r| r.unit.is_none() && r.unit_side.is_none()), "{:?}", o.ranked);
}

/// #3049: an onside kick on the list is the team that kicks off next: the
/// latest onside kick's only while nothing has scored since it.
#[test]
fn an_onside_kick_long_ago_is_not_the_next_kick() {
	use game_scenarios::Unit;
	let plays = plays();
	let onside = plays.iter().position(|p| p.text.as_deref().is_some_and(|t| t.contains("onside"))).unwrap();
	assert_eq!(plays[onside].possession.as_deref(), Some("NE"));
	// CHI scores after it: a field goal, shaped from a later CHI snap.
	let snap = (onside + 1..plays.len())
		.find(|&i| is_snap(&plays[i]) && plays[i].possession.as_deref() == Some("CHI"))
		.unwrap();
	// CHI is the home team.
	let kick = Play {
		play_type: Some("Field Goal Good".to_owned()),
		scoring: Some(true),
		text: Some("C.Santos 30 yard field goal is GOOD, Center-S.Daniels, Holder-T.Taylor.".to_owned()),
		home_score: Some(plays[snap - 1].home_score.unwrap_or(0) + 3),
		away_score: plays[snap - 1].away_score,
		..plays[snap].clone()
	};
	let o = just_after(&plays[..snap], kick);
	let ranked = o.ranked.iter().find(|r| r.scenario == Scenario::OnsideKick).unwrap();
	assert_eq!((ranked.unit, ranked.unit_side.as_deref()), (Some(Unit::SpecialTeams), Some("CHI")), "CHI kicks off next");
}

/// A fake is typed by what the ball did ("Rush", "Pass Reception"), its text
/// opening "(Field Goal formation)" or "(Punt formation)": the offense's run
/// or pass, never a kick, and a turnover on downs when it comes up short on
/// 4th down. The texts are ESPN's own, from 2025's regular season, with the
/// teams changed to this game's.
#[test]
fn a_fake_is_the_offenses_run_or_pass_never_a_kick() {
	let plays = plays();
	let punt = plays
		.iter()
		.position(|p| p.play_type.as_deref() == Some("Punt") && p.possession.as_deref() == Some("NE"))
		.unwrap();
	let shaped = |kind: &str, text: &str| Play {
		play_type: Some(kind.to_owned()),
		text: Some(text.to_owned()),
		..plays[punt].clone()
	};
	let at_the_punt = |play: Play| just_after(&plays[..punt], play);
	let happened = |o: &Offer, scenario: Scenario| o.just_happened.iter().any(|j| j.scenario == scenario);

	// MIN at DAL, 2025 (6 yards there): a fake field goal run is a run.
	let o = at_the_punt(shaped("Rush", "(Field Goal formation) B.Aubrey right end to CHI 23 for 26 yards (I.Pace)."));
	let (play, _) = big(&o).unwrap();
	assert_eq!((play.kind, play.size, play.by.as_str(), play.yards), (Kind::Run, Size::Big, "NE", 26));
	assert!(!happened(&o, Scenario::Kick), "a fake is no field goal");
	// NYG at NE, 2025: a fake that loses 13 on 4th down turns it over.
	let short = Play {
		down: Some(4),
		distance: Some(5),
		..shaped("Rush", "(Field Goal formation) J.Gillan right end to CHI 42 for -13 yards (J.Pharms).")
	};
	let o = at_the_punt(short);
	assert!(happened(&o, Scenario::Turnover), "a fake that comes up short is a turnover on downs");
	assert!(!happened(&o, Scenario::Kick));
	// KC at LV, 2025: a fake punt run is the offense's run.
	let o = at_the_punt(shaped("Rush", "(Punt formation) M.Araiza right end to CHI 22 for 32 yards (C.Heyward)."));
	assert_eq!(big(&o).map(|(p, _)| (p.kind, p.by.as_str())), Some((Kind::Run, "NE")));
	// CIN at HOU, 2025: a flag before the kick, then the kick, is a field goal.
	let kicked = shaped(
		"Field Goal Good",
		"(Field Goal formation) PENALTY on CHI-D.Risner, Offside, 5 yards. J.Slye 40 yard field goal is GOOD, Center-L.Rhodes, Holder-R.Sanchez.",
	);
	assert!(happened(&at_the_punt(kicked), Scenario::Kick));
}

/// ESPN's play text spells six teams otherwise than its header: HOU is
/// "HST" in "PENALTY on HST" and "recovered by HST-J.Watson". Each is read
/// as the header's team, so a flag says "HOU" and a fumble HOU fell on is
/// no turnover. The texts are ESPN's own, from 2025's regular season.
#[test]
fn a_team_the_play_text_spells_otherwise_is_the_headers_team() {
	let plays = plays();
	let punt = plays
		.iter()
		.position(|p| p.play_type.as_deref() == Some("Punt") && p.possession.as_deref() == Some("NE"))
		.unwrap();
	let shaped = |kind: &str, text: &str| Play {
		play_type: Some(kind.to_owned()),
		text: Some(text.to_owned()),
		possession: Some("HOU".to_owned()),
		..plays[punt].clone()
	};
	let just = |play: Play| {
		let seen = wall(&play) + TimeDelta::seconds(10);
		let mut log = PlayLog::default();
		log.apply(GAME, 1, &seen.to_rfc3339_opts(SecondsFormat::Secs, true), std::slice::from_ref(&play));
		let entries: Vec<Entry> = log.entries().to_vec();
		let g = Game {
			home: "IND".to_owned(),
			away: "HOU".to_owned(),
			..game(Some("IND"), status(GameState::In, Some(&play)), None)
		};
		offer(&g, &entries, pacific(seen + TimeDelta::seconds(5)), Mode::Full, Watching::Unknown)
			.just_happened
			.into_iter()
			.map(|j| (j.scenario, j.line))
			.collect::<Vec<_>>()
	};
	// HOU at IND, 2025.
	let flag = just(shaped("Penalty", "(Punt formation) PENALTY on HST, Delay of Game, 4 yards, enforced at HST 9 - No Play."));
	assert!(flag.contains(&(Scenario::Penalty, "Flag on HOU, delay of game".to_owned())), "{flag:?}");
	// A fumble HOU fell on is HOU's ball still.
	let own = just(Play {
		down: Some(1),
		..shaped(
			"Fumble Recovery (Own)",
			"C.Stroud sacked at HST 30 for -6 yards (D.Buckner). FUMBLES (D.Buckner), RECOVERED by HST-N.Collins at HST 28.",
		)
	});
	assert!(own.iter().all(|(s, _)| *s != Scenario::Turnover), "{own:?}");
}

/// A foul after the play, "enforced between downs" (a celebration), does not
/// undo the return before it, as one during the return does. The flag's
/// words are ESPN's own (TEN at IND, 2025).
#[test]
fn a_flag_between_downs_leaves_the_return() {
	let plays = plays();
	let punt = plays
		.iter()
		.position(|p| p.play_type.as_deref() == Some("Punt") && p.possession.as_deref() == Some("NE"))
		.unwrap();
	let returned = Play {
		text: Some("M.Wishnowsky punts 50 yards to CHI 10, Center-J.Ashby. K.Raymond to NE 40 for 50 yards (D.Muasau). PENALTY on CHI, Unsportsmanlike Conduct, 15 yards, enforced between downs.".to_owned()),
		..plays[punt].clone()
	};
	let o = just_after(&plays[..punt], returned);
	assert_eq!(big(&o).map(|(p, _)| (p.kind, p.yards, p.by.as_str())), Some((Kind::PuntReturn, 50, "CHI")));
}

/// #3334: a blocked kick the other side runs back is its return, and a
/// special-teams touchdown is big whatever its yards (C967). The texts are
/// ESPN's own, from 2025's regular season, with the teams changed to this
/// game's.
#[test]
fn a_blocked_kick_run_back_is_the_returners_and_a_special_teams_score_is_big() {
	let plays = plays();
	let punt = plays
		.iter()
		.position(|p| p.play_type.as_deref() == Some("Punt") && p.possession.as_deref() == Some("NE"))
		.unwrap();
	let shaped = |kind: &str, text: &str| Play {
		play_type: Some(kind.to_owned()),
		text: Some(text.to_owned()),
		..plays[punt].clone()
	};
	let at_the_punt = |play: Play| just_after(&plays[..punt], play);

	// PHI at TB: a blocked punt run back 35 yards for a touchdown.
	let o = at_the_punt(shaped(
		"Blocked Punt Touchdown",
		"R.Dixon punt is BLOCKED by C.Latu, Center-E.Deckers, RECOVERED by CHI-S.Brown at NE 35. S.Brown for 35 yards, TOUCHDOWN. J.Elliott extra point is GOOD, Center-C.Hughlett, Holder-B.Mann.",
	));
	let (play, line) = big(&o).unwrap();
	assert_eq!(
		(play.kind, play.size, play.by.as_str(), play.yards, play.scored, play.good),
		(Kind::PuntReturn, Size::Big, "CHI", 35, true, Some(false))
	);
	assert_eq!(line, "CHI 35-yard punt return, touchdown");
	// TEN at IND: 14 yards, but a special-teams touchdown, so big.
	let o = at_the_punt(shaped(
		"Blocked Punt Touchdown",
		"R.Sanchez punt is BLOCKED by J.Chestnut, Center-L.Rhodes, RECOVERED by CHI-O.Oladejo at NE 14. O.Oladejo for 14 yards, TOUCHDOWN. NE-R.Sanchez was injured during the play. PENALTY on CHI, Unsportsmanlike Conduct, 15 yards, enforced between downs. J.Slye extra point is GOOD, Center-M.Cox, Holder-J.Stonehouse.",
	));
	let (play, _) = big(&o).unwrap();
	assert_eq!((play.kind, play.size, play.by.as_str(), play.scored), (Kind::PuntReturn, Size::Big, "CHI", true));
	// LV at IND: blocked, and the kickers fell on it: nothing returned.
	assert!(big(&at_the_punt(shaped(
		"Blocked Punt",
		"A.Cole punt is BLOCKED by S.Olubi, Center-J.Bobenmoyer, recovered by NE-I.Thomas at NE 8."
	)))
	.is_none());
	// MIN at DET: a blocked field goal run back 41 yards is a kick return.
	let o = at_the_punt(shaped(
		"Blocked Field Goal",
		"J.Bates 45 yard field goal is BLOCKED (L.Rodriguez), Center-H.Hatten, Holder-J.Fox, RECOVERED by CHI-I.Rodgers at CHI 33. I.Rodgers pushed ob at NE 26 for 41 yards (J.Fox).",
	));
	let (play, _) = big(&o).unwrap();
	assert_eq!((play.kind, play.size, play.by.as_str(), play.yards), (Kind::KickReturn, Size::Huge, "CHI", 41));
	// SF at SEA: run back 8, no score: not big.
	assert!(big(&at_the_punt(shaped(
		"Blocked Field Goal",
		"J.Moody 36 yard field goal is BLOCKED (J.Love), Center-J.Weeks, Holder-T.Morstead, RECOVERED by CHI-D.Lawrence at CHI 23. D.Lawrence to CHI 31 for 8 yards (C.McKivitz).",
	)))
	.is_none());
}

/// A big play is read as the play stood (C967's "a turnover a review
/// reversed is read only as it stood", for every kind of play): after a
/// review that reversed it, only ESPN's ruling after "REVERSED." counts. A
/// fumble before the kick is the kicking team's, from scrimmage, and a return
/// touchdown under an accepted flag on the returners was not the play.
#[test]
fn a_big_play_is_the_play_as_it_stood() {
	let plays = plays();
	let punt = plays
		.iter()
		.position(|p| p.play_type.as_deref() == Some("Punt") && p.possession.as_deref() == Some("NE"))
		.unwrap();
	let pass = plays
		.iter()
		.position(|p| p.play_type.as_deref() == Some("Pass Reception") && p.possession.as_deref() == Some("NE"))
		.unwrap();
	let at = |k: usize, kind: &str, text: &str| {
		big(&just_after(
			&plays[..k],
			Play {
				play_type: Some(kind.to_owned()),
				text: Some(text.to_owned()),
				..plays[k].clone()
			},
		))
		.map(|(p, line)| (p.kind, p.by.clone(), line.to_owned()))
	};
	// A punt return touchdown a review took back is no big play.
	assert_eq!(
		at(
			punt,
			"Punt Return Touchdown",
			"R.Dixon punts 45 yards to CHI 10, Center-E.Deckers. K.Raymond to NE 0 for 90 yards, TOUCHDOWN. The Replay Official reviewed the runner was out of bounds ruling, and the play was REVERSED. R.Dixon punts 45 yards to CHI 10, Center-E.Deckers. K.Raymond pushed ob at CHI 30 for 20 yards (R.Dixon)."
		)
		.map(|(kind, ..)| kind),
		None,
		"the touchdown never stood, and a reversed return's yards are not read"
	);
	// A pass reversed into an interception is a turnover, so big.
	let reversed_into = at(
		pass,
		"Pass Incompletion",
		"D.Maye pass incomplete short left to H.Henry. The Replay Official reviewed the pass incomplete ruling, and the play was REVERSED. D.Maye pass short left intended for H.Henry INTERCEPTED by K.Byard at NE 45. K.Byard to NE 40 for 5 yards (H.Henry).",
	);
	assert_eq!(reversed_into.map(|(kind, by, _)| (kind, by)), Some((Kind::InterceptionReturn, "CHI".to_owned())));
	// An interception a review reversed is no big play.
	assert_eq!(
		at(
			pass,
			"Interception Return",
			"D.Maye pass short left intended for H.Henry INTERCEPTED by K.Byard at NE 45. K.Byard to NE 40 for 5 yards. The Replay Official reviewed the interception ruling, and the play was REVERSED. D.Maye pass incomplete short left to H.Henry."
		),
		None
	);
	// The punter's own fumble, which his team fell on, turned nothing over.
	assert_eq!(
		at(
			punt,
			"Punt",
			"R.Dixon FUMBLES (Aborted) at NE 30, recovered by NE-R.Dixon at NE 28. R.Dixon punts 40 yards to CHI 32, fair catch by K.Raymond."
		),
		None
	);
	// The punter's fumble the other side fell on is a turnover, so big.
	let lost = at(punt, "Punt", "R.Dixon FUMBLES (Aborted) at NE 30, RECOVERED by CHI-S.Brown at NE 28.");
	assert_eq!(lost.map(|(kind, by, _)| (kind, by)), Some((Kind::FumbleReturn, "CHI".to_owned())));
	// A touchdown reversed into an interception in the end zone scored nothing.
	let pass_td = just_after(
		&plays[..pass],
		Play {
			play_type: Some("Passing Touchdown".to_owned()),
			text: Some("D.Maye pass short left to H.Henry for 10 yards, TOUCHDOWN. The Replay Official reviewed the pass completion ruling, and the play was REVERSED. D.Maye pass short left intended for H.Henry INTERCEPTED by K.Byard in End Zone. Touchback.".to_owned()),
			..plays[pass].clone()
		},
	);
	let (p, line) = big(&pass_td).unwrap();
	assert_eq!((p.kind, p.by.as_str(), p.scored), (Kind::InterceptionReturn, "CHI", false), "{line}");
	// A flag on the try leaves the return touchdown before it standing, its
	// yards and size with it.
	let tried = just_after(
		&plays[..punt],
		Play {
			play_type: Some("Punt Return Touchdown".to_owned()),
			text: Some("R.Dixon punts 45 yards to CHI 10, Center-E.Deckers. K.Raymond to NE 0 for 90 yards, TOUCHDOWN. C.Santos extra point is GOOD, Center-S.Daly, Holder-T.Taylor. PENALTY on CHI-K.Raymond, Taunting, 15 yards, enforced on the kickoff.".to_owned()),
			..plays[punt].clone()
		},
	);
	let (p, line) = big(&tried).unwrap();
	assert_eq!(
		(p.kind, p.by.as_str(), p.yards, p.size, p.scored),
		(Kind::PuntReturn, "CHI", 90, Size::Memorable, true),
		"{line}"
	);
	assert_eq!(line, "CHI 90-yard punt return, touchdown");
	// A kick in words read nowhere else ("free kick") is still a kick: the
	// returners' muff the kickers fell on is turned over.
	let free = at(
		punt,
		"Kickoff",
		"R.Dixon free kick 60 yards from NE 20 to CHI 20. K.Raymond MUFFS catch, RECOVERED by NE-J.Elliss at CHI 20.",
	);
	assert_eq!(free.map(|(kind, by, _)| (kind, by)), Some((Kind::FumbleReturn, "NE".to_owned())));
	// A return touchdown under an accepted flag on the returners is no big play.
	assert_eq!(
		at(
			punt,
			"Punt Return Touchdown",
			"R.Dixon punts 45 yards to CHI 10, Center-E.Deckers. K.Raymond to NE 0 for 90 yards, TOUCHDOWN. PENALTY on CHI-J.Brisker, Illegal Block Above the Waist, 10 yards, enforced at CHI 30."
		),
		None
	);
}

/// #1103: the try's text never changes its touchdown's reading. Family:
/// every try outcome (made or missed, either kind; a No-Play flag on either
/// side; a fumble either side recovers; an interception; a sack; a review
/// that reversed it) after every kind of touchdown (run, pass, kick, onside
/// kick, punt, blocked-punt, blocked-field-goal, interception and fumble
/// returns, big and not). Each is read as the same touchdown with its try
/// still to come, by every scenario but the four that read the try itself
/// (`in.two-point`, `in.kick`, `in.penalty`, `in.injury`).
#[test]
fn the_try_never_changes_its_touchdowns_reading() {
	let plays = plays();
	let find = |kind: &str, team: &str| {
		plays
			.iter()
			.position(|p| p.play_type.as_deref() == Some(kind) && p.possession.as_deref() == Some(team))
			.unwrap()
	};
	let pass = find("Pass Reception", "NE");
	let punt = find("Punt", "NE");
	let kickoff = find("Kickoff", "CHI");
	let pick = plays.iter().position(|p| p.text.as_deref().is_some_and(|t| t.contains("INTERCEPTED"))).unwrap();
	assert_eq!(plays[pick].possession.as_deref(), Some("NE"));
	// (base play, ESPN's type, the touchdown's text, who scored, whether big)
	let touchdowns = [
		(pass, "Rushing Touchdown", "R.Stevenson left end for 60 yards, TOUCHDOWN.", "NE", true),
		(pass, "Rushing Touchdown", "R.Stevenson up the middle for 2 yards, TOUCHDOWN.", "NE", false),
		(pass, "Passing Touchdown", "(Shotgun) D.Maye pass deep right to S.Diggs for 62 yards, TOUCHDOWN.", "NE", true),
		(pass, "Passing Touchdown", "D.Maye pass short left to H.Henry for 8 yards, TOUCHDOWN.", "NE", false),
		(kickoff, "Kickoff Return Touchdown", "C.Santos kicks 65 yards from CHI 35 to NE 0. C.Kiner to CHI 0 for 100 yards, TOUCHDOWN.", "NE", true),
		(
			kickoff,
			"Kickoff Return Touchdown",
			"C.Santos kicks onside 12 yards from CHI 35 to CHI 47. C.Kiner to CHI 0 for 47 yards, TOUCHDOWN.",
			"NE",
			true,
		),
		(punt, "Punt Return Touchdown", "R.Dixon punts 45 yards to CHI 10, Center-E.Deckers. K.Raymond to NE 0 for 90 yards, TOUCHDOWN.", "CHI", true),
		(
			punt,
			"Blocked Punt Touchdown",
			"R.Dixon punt is BLOCKED by C.Latu, Center-E.Deckers, RECOVERED by CHI-S.Brown at NE 35. S.Brown for 35 yards, TOUCHDOWN.",
			"CHI",
			true,
		),
		(
			punt,
			"Blocked Field Goal Touchdown",
			"J.Bates 45 yard field goal is BLOCKED (L.Rodriguez), Center-H.Hatten, Holder-J.Fox, RECOVERED by CHI-I.Rodgers at CHI 33. I.Rodgers to NE 0 for 67 yards, TOUCHDOWN.",
			"CHI",
			true,
		),
		(
			pick,
			"Interception Return Touchdown",
			"(Shotgun) D.Maye pass short left intended for H.Henry INTERCEPTED by K.Byard at NE 45. K.Byard to NE 0 for 45 yards, TOUCHDOWN.",
			"CHI",
			true,
		),
		(
			pass,
			"Fumble Return Touchdown",
			"D.Maye pass short right to H.Henry to NE 40 for 5 yards (T.Edmunds). H.Henry FUMBLES (T.Edmunds), RECOVERED by CHI-K.Byard at NE 40. K.Byard to NE 0 for 40 yards, TOUCHDOWN.",
			"CHI",
			true,
		),
	];
	// Each try by the scorer `s` against `d`.
	let tries = |s: &str, d: &str| -> Vec<String> {
		let two = "TWO-POINT CONVERSION ATTEMPT.";
		vec![
			"A.Kicker extra point is GOOD, Center-A.Snapper, Holder-A.Holder.".to_owned(),
			"A.Kicker extra point is No Good, Wide Right, Center-A.Snapper, Holder-A.Holder.".to_owned(),
			"A.Kicker kick is No Good, Wide Left, Center-A.Snapper, Holder-A.Holder.".to_owned(),
			format!("A.Kicker extra point is BLOCKED (B.Rusher), Center-A.Snapper, Holder-A.Holder, RECOVERED by {d}-B.Rusher at {s} 40. B.Rusher to {s} 0 for 60 yards."),
			format!("{two} A.Passer pass to A.Catcher is complete. ATTEMPT SUCCEEDS."),
			format!("{two} A.Passer pass to A.Catcher is incomplete. ATTEMPT FAILS."),
			format!("A.Kicker extra point is GOOD, Center-A.Snapper, Holder-A.Holder. PENALTY on {d}-B.Lineman, Defensive Offside, 1 yard, enforced at {d} 15 - No Play."),
			format!("{two} A.Passer pass to A.Catcher is complete. ATTEMPT SUCCEEDS. PENALTY on {s}-A.Lineman, False Start, 5 yards, enforced at {d} 2 - No Play."),
			format!("{two} A.Runner rush up the middle. A.Runner FUMBLES (B.Tackler), RECOVERED by {d}-B.Tackler at {d} 3. B.Tackler to {s} 0 for 97 yards. ATTEMPT FAILS."),
			format!("{two} A.Runner rush up the middle. A.Runner FUMBLES (B.Tackler), recovered by {s}-A.Runner at {d} 4. ATTEMPT FAILS."),
			format!("{two} A.Passer pass short left intended for A.Catcher INTERCEPTED by B.Safety at {d} 0. B.Safety to {s} 0 for 100 yards. ATTEMPT FAILS."),
			format!("{two} A.Passer sacked at {d} 10 for -8 yards (B.Rusher). ATTEMPT FAILS."),
			format!("{two} A.Passer pass to A.Catcher is complete. ATTEMPT SUCCEEDS. The Replay Official reviewed the pass completion ruling, and the play was REVERSED. A.Passer pass to A.Catcher is incomplete. ATTEMPT FAILS."),
		]
	};
	let reads_the_try = ["in.two-point", "in.kick", "in.penalty", "in.injury"];
	let read = |k: usize, kind: &str, text: &str| {
		let shown = Play {
			play_type: Some(kind.to_owned()),
			text: Some(text.to_owned()),
			..plays[k].clone()
		};
		let seen = wall(&shown) + TimeDelta::seconds(10);
		let mut held = plays[..k].to_vec();
		held.push(shown.clone());
		let mut log = PlayLog::default();
		log.apply(GAME, 1, &seen.to_rfc3339_opts(SecondsFormat::Secs, true), &held);
		let g = game(Some("NE"), status(GameState::In, Some(&shown)), None);
		scenario_engine::moment_log(&g, log.entries(), seen + TimeDelta::seconds(5))
			.into_iter()
			.map(|logged| logged.happened)
			.filter(|h| h.play_id == shown.id && !reads_the_try.contains(&h.scenario.id()))
			.collect::<Vec<_>>()
	};
	let mut cases = 0;
	for (k, kind, touchdown, s, is_big) in touchdowns {
		let d = if s == "NE" { "CHI" } else { "NE" };
		let alone = read(k, kind, touchdown);
		let big_play = alone.iter().find(|h| h.scenario == Scenario::BigPlay).and_then(|h| h.big_play.clone());
		assert_eq!(big_play.is_some(), is_big, "{touchdown}: {alone:?}");
		assert!(big_play.is_none_or(|b| b.scored && b.by == s), "{touchdown}: {alone:?}");
		// The onside kick is read too, so its case is not vacuous.
		assert_eq!(
			alone.iter().any(|h| h.scenario.id() == "in.onside-kick"),
			touchdown.contains("onside"),
			"{touchdown}: {alone:?}"
		);
		for try_text in tries(s, d) {
			let text = format!("{touchdown} {try_text}");
			assert_eq!(read(k, kind, &text), alone, "{text}");
			cases += 1;
		}
	}
	assert_eq!(cases, 143);
}
