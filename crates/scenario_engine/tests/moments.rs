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

	// The same play with the return stopped short of 20 is no big play.
	let short = Play {
		text: Some("(Shotgun) D.Maye pass short left intended for H.Henry INTERCEPTED by K.Byard at NE 45. K.Byard to NE 33 for 12 yards.".to_owned()),
		..plays[pick].clone()
	};
	assert!(big(&just_after(&plays[..pick], short)).is_none());

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
	assert!(big(&just_after(&plays[..punt], muffed)).is_none());
	let taken_away = Play {
		text: Some(
			"(Shotgun) D.Maye pass deep right to S.Diggs for 62 yards, TOUCHDOWN NULLIFIED by Penalty. PENALTY on NE-M.Onwenu, Offensive Holding, 10 yards, enforced at NE 38."
				.to_owned(),
		),
		..plays[pass].clone()
	};
	let o = just_after(&plays[..pass], taken_away);
	assert!(big(&o).is_none_or(|(play, _)| !play.scored), "a touchdown taken away did not score");
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
