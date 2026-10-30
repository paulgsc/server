//! Every string an offer carries: the strip, each scenario's line, how long
//! ago (SE4). In the person's terms, as the canvas board `Say it: what can be
//! happening now` shows them ("3rd and 8, down 4", "1:52 left, 1 timeout"):
//! never a field name, an id, or ESPN's spelling of a play type.

use crate::{
	read::{SoFar, Try},
	Game, Strip,
};
use chrono::{DateTime, Utc};
use nfl_feed::{GameState, Weather};

/// `1st`, `2nd`, `3rd`, `4th`.
pub(crate) fn ordinal(n: i64) -> String {
	let suffix = match n {
		1 => "st",
		2 => "nd",
		3 => "rd",
		_ => "th",
	};
	[n.to_string().as_str(), suffix].concat()
}

/// A quarter, or overtime.
fn period(period: i64) -> String {
	if period >= 5 {
		String::from("overtime")
	} else {
		["the ", &ordinal(period)].concat()
	}
}

/// `3rd and 8`, or `1st and goal` when the line to gain is the goal line.
pub(crate) fn down_distance(down: i64, distance: i64, to_endzone: Option<i64>) -> String {
	if to_endzone.is_some_and(|y| distance >= y) {
		[ordinal(down).as_str(), " and goal"].concat()
	} else {
		[ordinal(down), String::from(" and "), distance.to_string()].concat()
	}
}

/// `at the CHI 35`: the side of the field the ball is on.
pub(crate) fn spot(to_endzone: i64, offense: &str, defense: &str) -> String {
	match to_endzone.cmp(&50) {
		std::cmp::Ordering::Equal => String::from("at midfield"),
		std::cmp::Ordering::Greater => ["at the ", offense, " ", &(100 - to_endzone).to_string()].concat(),
		std::cmp::Ordering::Less => ["at the ", defense, " ", &to_endzone.to_string()].concat(),
	}
}

/// `up 4`, `down 4`, `tied`.
pub(crate) fn margin(margin: i64) -> String {
	match margin.cmp(&0) {
		std::cmp::Ordering::Equal => String::from("tied"),
		std::cmp::Ordering::Greater => ["up ", &margin.to_string()].concat(),
		std::cmp::Ordering::Less => ["down ", &(-margin).to_string()].concat(),
	}
}

/// `CHI 13, NE 7`, the side ahead first; `Tied at 7`; `No score yet`.
pub(crate) fn score(game: &Game, (away, home): (i64, i64)) -> String {
	match away.cmp(&home) {
		std::cmp::Ordering::Equal if away == 0 => String::from("No score yet"),
		std::cmp::Ordering::Equal => ["Tied at ", &away.to_string()].concat(),
		std::cmp::Ordering::Greater => [game.away.as_str(), " ", &away.to_string(), ", ", &game.home, " ", &home.to_string()].concat(),
		std::cmp::Ordering::Less => [game.home.as_str(), " ", &home.to_string(), ", ", &game.away, " ", &away.to_string()].concat(),
	}
}

/// `30-24`, the winner's points first.
fn final_score((away, home): (i64, i64)) -> String {
	[away.max(home).to_string(), String::from("-"), away.min(home).to_string()].concat()
}

/// `1:52 left in the 4th`, `Halftime`, `End of the 1st`.
pub(crate) fn game_clock(at_period: i64, clock: Option<&str>, seconds: Option<i64>) -> String {
	match (clock, seconds) {
		(_, Some(0)) if at_period == 2 => String::from("Halftime"),
		(_, Some(0)) => ["End of ", &period(at_period)].concat(),
		(Some(clock), _) => [clock, " left in ", &period(at_period)].concat(),
		(None, _) => ["In ", &period(at_period)].concat(),
	}
}

/// `Kickoff in 2 h 10 min`.
pub(crate) fn kickoff_in(seconds: i64) -> String {
	if seconds <= 0 {
		return String::from("Kickoff any minute");
	}
	let minutes = (seconds + 59) / 60;
	let (hours, minutes) = (minutes / 60, minutes % 60);
	match (hours, minutes) {
		(0, m) => ["Kickoff in ", &m.to_string(), " min"].concat(),
		(h, 0) => ["Kickoff in ", &h.to_string(), " h"].concat(),
		(h, m) => ["Kickoff in ", &h.to_string(), " h ", &m.to_string(), " min"].concat(),
	}
}

/// `just now`, `40 seconds ago`, `2 minutes ago`.
pub(crate) fn ago(seconds: i64) -> String {
	match seconds {
		..15 => String::from("just now"),
		15..60 => [seconds.to_string().as_str(), " seconds ago"].concat(),
		60..120 => String::from("a minute ago"),
		_ => [(seconds / 60).to_string().as_str(), " minutes ago"].concat(),
	}
}

/// `NE, ` before a line about the team with the ball, unless it is the team
/// the game is read for.
pub(crate) fn who(team: &str, us: Option<&str>) -> String {
	if us == Some(team) {
		String::new()
	} else {
		[team, ", "].concat()
	}
}

/// `3rd and 8, down 4` when the side we want has the ball; `NE, 3rd and 8,
/// up 4` otherwise. The margin is the team with the ball's.
pub(crate) fn at_the_snap(offense: &str, us: Option<&str>, situation: &str, margin_now: Option<i64>) -> String {
	let mut line = [who(offense, us), situation.to_owned()].concat();
	if let Some(m) = margin_now {
		line.push_str(", ");
		line.push_str(&margin(m));
	}
	line
}

/// `1:52 left, 1 timeout`, for the team with the ball.
pub(crate) fn against_the_clock(clock: &str, offense: Option<&str>, us: Option<&str>, timeouts: Option<i64>, end_of_game: bool) -> String {
	let mut line = [clock, " left"].concat();
	match (offense, timeouts) {
		(Some(team), Some(left)) => {
			line.push_str(", ");
			if us != Some(team) {
				line.push_str(team);
				line.push_str(" has ");
			}
			line.push_str(&match left {
				0 => String::from("no timeouts"),
				1 => String::from("1 timeout"),
				n => [n.to_string().as_str(), " timeouts"].concat(),
			});
		}
		_ => line.push_str(if end_of_game { " in the game" } else { " in the half" }),
	}
	line
}

/// `NE's first drive`.
pub(crate) fn first_drive(team: &str) -> String {
	[team, "'s first drive"].concat()
}

/// `A 43-yard try, up 6`; `NE, a 43-yard try, down 6`.
pub(crate) fn field_goal_try(offense: &str, us: Option<&str>, yards: i64, margin_now: Option<i64>) -> String {
	let mut line = [who(offense, us).as_str(), "a ", &yards.to_string(), "-yard try"].concat();
	if let Some(m) = margin_now {
		line.push_str(", ");
		line.push_str(&margin(m));
	}
	capitalized(&line)
}

/// `line` with its first letter upper-cased.
fn capitalized(line: &str) -> String {
	let mut chars = line.chars();
	chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

/// `NE touchdown, down 1 before the try`.
pub(crate) fn before_the_try(team: &str, margin_now: Option<i64>) -> String {
	margin_now.map_or_else(
		|| [team, " touchdown, before the try"].concat(),
		|m| [team, " touchdown, ", &margin(m), " before the try"].concat(),
	)
}

/// `Kick it or go for two, down 1`.
pub(crate) fn kick_or_go(margin_now: Option<i64>) -> String {
	margin_now.map_or_else(|| String::from("Kick it or go for two"), |m| ["Kick it or go for two, ", &margin(m)].concat())
}

/// `Down 3 after the NE touchdown`, `Up 4 after the CHI field goal`.
pub(crate) fn lead_change(margin_now: i64, by: &str, how: &str) -> String {
	let state = if margin_now < 0 {
		["Down ", &(-margin_now).to_string()].concat()
	} else {
		["Up ", &margin_now.to_string()].concat()
	};
	[state.as_str(), " after the ", by, " ", how].concat()
}

/// What kind of score a scoring play was.
pub(crate) const fn score_kind(touchdown: bool, field_goal: bool) -> &'static str {
	if touchdown {
		"touchdown"
	} else if field_goal {
		"field goal"
	} else {
		"score"
	}
}

/// `CHI kicks off, NE receives`.
pub(crate) fn kicks_off(kicking: &str, receiving: Option<&str>) -> String {
	receiving.map_or_else(|| [kicking, " kicks off"].concat(), |receiving| [kicking, " kicks off, ", receiving, " receives"].concat())
}

/// `NE at CHI`, the line for a game before kickoff.
pub(crate) fn matchup(game: &Game) -> String {
	[game.away.as_str(), " at ", &game.home].concat()
}

/// `NE at CHI, kickoff in 2 h 10 min`.
pub(crate) fn before_kickoff(game: &Game, seconds: i64) -> String {
	let mut lower = kickoff_in(seconds);
	lower.replace_range(..1, "k");
	[matchup(game), String::from(", "), lower].concat()
}

/// `52°, mostly cloudy, gusts to 23 mph, 40% chance of rain`.
pub(crate) fn forecast(weather: Option<&Weather>) -> String {
	let Some(weather) = weather else {
		return String::from("No forecast yet");
	};
	let mut parts: Vec<String> = Vec::new();
	if let Some(t) = weather.temperature {
		parts.push([t.to_string().as_str(), "°"].concat());
	}
	if let Some(display) = &weather.display {
		parts.push(display.to_lowercase());
	}
	if let Some(gust) = weather.gust.filter(|g| *g >= 15) {
		parts.push(["gusts to ", &gust.to_string(), " mph"].concat());
	}
	if let Some(chance) = weather.precipitation.filter(|p| *p >= 20) {
		parts.push([chance.to_string().as_str(), "% chance of rain"].concat());
	}
	if parts.is_empty() {
		String::from("No forecast yet")
	} else {
		parts.join(", ")
	}
}

/// `CHI at home`, `CHI on the road`.
pub(crate) fn venue(game: &Game, want: Option<&str>) -> String {
	match want {
		Some(team) if team == game.home => [team, " at home"].concat(),
		Some(team) if team == game.away => [team, " on the road at ", &game.home].concat(),
		_ => matchup(game),
	}
}

/// `SF's own game`, `SF wants CHI to win`.
pub(crate) fn stakes(team: &str, want: &str) -> String {
	if team == want {
		[team, "'s own game"].concat()
	} else {
		[team, " wants ", want, " to win"].concat()
	}
}

/// `CHI won 30-24`, `Tied 24-24`.
pub(crate) fn result(game: &Game, score: (i64, i64)) -> String {
	let (away, home) = score;
	if away == home {
		return ["Tied ", &final_score(score)].concat();
	}
	let winner = if away > home { &game.away } else { &game.home };
	[winner.as_str(), " won ", &final_score(score)].concat()
}

/// `CHI won 30-24, as SF wanted`.
pub(crate) fn result_for(game: &Game, score: (i64, i64), team: &str, want: Option<&str>, our_way: Option<bool>) -> String {
	let result = result(game, score);
	match (want, our_way) {
		(Some(want), Some(true)) if want != team => [result.as_str(), ", as ", team, " wanted"].concat(),
		(Some(want), Some(false)) if want != team => [result.as_str(), ", not as ", team, " wanted"].concat(),
		_ => result,
	}
}

/// `What CHI's win does to SF's paths`.
pub(crate) fn paths(team: &str, want: &str, our_way: Option<bool>) -> String {
	let what = match our_way {
		Some(true) => "win",
		Some(false) => "loss",
		None => "tie",
	};
	["What ", want, "'s ", what, " does to ", team, "'s paths"].concat()
}

/// `Final: CHI 30, NE 24`.
pub(crate) fn final_line(game: &Game, score_now: (i64, i64)) -> String {
	["Final: ", &score(game, score_now)].concat()
}

/// `You watched CHI win 30-24`, `You skipped it, and CHI lost 24-30`.
pub(crate) fn you(want: &str, watched: bool, late_margin: Option<i64>, our_way: bool, score: (i64, i64)) -> String {
	let (high, low) = (score.0.max(score.1).to_string(), score.0.min(score.1).to_string());
	let outcome = if our_way {
		[want, " win ", &high, "-", &low].concat()
	} else {
		[want, " lose ", &low, "-", &high].concat()
	};
	match (watched, late_margin) {
		(true, Some(m)) => ["You tuned in with ", want, " ", &margin(m), ", and watched ", &outcome].concat(),
		(true, None) => ["You watched ", &outcome].concat(),
		(false, _) => {
			let past = if our_way {
				[want, " won ", &high, "-", &low].concat()
			} else {
				[want, " lost ", &low, "-", &high].concat()
			};
			["You skipped it, and ", &past].concat()
		}
	}
}

// What just happened.

/// `NE sacked on 4th and 5`.
pub(crate) fn sacked(offense: &str, situation: Option<&str>) -> String {
	situation.map_or_else(|| [offense, " sacked"].concat(), |situation| [offense, " sacked on ", situation].concat())
}

/// `NE threw an interception`, `NE lost a fumble`, `NE turned it over on
/// downs`.
pub(crate) fn turned_over(offense: &str, how: &str) -> String {
	[offense, " ", how].concat()
}

/// `Flag on NE, false start`.
pub(crate) fn flag(team: &str, foul: Option<&str>) -> String {
	foul.map_or_else(|| ["Flag on ", team].concat(), |foul| ["Flag on ", team, ", ", foul].concat())
}

/// `CHI gained 58`, `NE 48-yard touchdown`.
pub(crate) fn big_play(offense: &str, yards: i64, touchdown: bool) -> String {
	if touchdown {
		[offense, " ", &yards.to_string(), "-yard touchdown"].concat()
	} else {
		[offense, " gained ", &yards.to_string()].concat()
	}
}

/// `CHI field goal from 43, good`.
pub(crate) fn kicked(offense: &str, what: &str, yards: Option<i64>, how: &str) -> String {
	yards.map_or_else(
		|| [offense, " ", what, ", ", how].concat(),
		|yards| [offense, " ", what, " from ", &yards.to_string(), ", ", how].concat(),
	)
}

/// `NE touchdown, kicked the extra point`; `NE touchdown, the try to come`.
pub(crate) fn touchdown(offense: &str, try_: Try) -> String {
	let how = match try_ {
		Try::ToCome => ", the try to come",
		Try::Kicked => ", kicked the extra point",
		Try::WentForTwo => ", went for two",
	};
	[offense, " touchdown", how].concat()
}

/// `3rd and 8: converted`.
pub(crate) fn down_result(situation: &str, result: Option<&str>) -> String {
	result.map_or_else(|| situation.to_owned(), |result| [situation, ": ", result].concat())
}

/// `NE in the red zone: touchdown`.
pub(crate) fn red_zone_trip(offense: &str, result: &str) -> String {
	[offense, " in the red zone: ", result].concat()
}

/// `A NE player is hurt`, `A NE player is back`.
pub(crate) fn hurt(team: Option<&str>, back: bool) -> String {
	let what = if back { " player is back" } else { " player is hurt" };
	team.map_or_else(|| ["A", what].concat(), |team| ["A ", team, what].concat())
}

/// `NE's first drive: field goal`.
pub(crate) fn first_drive_ended(team: &str, result: &str) -> String {
	[first_drive(team).as_str(), ": ", result].concat()
}

/// `Ran on 3rd and 1`.
pub(crate) fn called(ran: bool, situation: &str) -> String {
	[if ran { "Ran on " } else { "Threw on " }, situation].concat()
}

/// The live strip.
pub(crate) fn strip(game: &Game, so_far: &SoFar<'_>, now: DateTime<Utc>) -> Strip {
	if so_far.over() {
		return Strip {
			clock: String::from("Final"),
			possession: None,
			down: None,
			score: Some(score(game, so_far.score())),
		};
	}
	if so_far.not_played() {
		return Strip {
			clock: String::from("Not played"),
			possession: None,
			down: None,
			score: None,
		};
	}
	if so_far.plays.is_empty() && game.status.state != Some(GameState::In) {
		return Strip {
			clock: kickoff_in((game.kickoff - now).num_seconds()),
			possession: None,
			down: None,
			score: None,
		};
	}
	let latest = so_far.plays.last().map(|seen| seen.play);
	let at_period = game.status.period.or_else(|| latest.and_then(|p| p.period)).unwrap_or(1);
	let clock = game.status.clock.as_deref().or_else(|| latest.and_then(|p| p.clock.as_deref()));
	let seconds = clock.and_then(crate::read::clock_seconds);
	let situation = game.situation.as_ref();
	let offense = situation.and_then(|s| s.possession.as_deref());
	let down = situation.and_then(|s| {
		let (down, distance) = (s.down?, s.distance?);
		let mut line = down_distance(down, distance, s.yards_to_endzone);
		if let (Some(y), Some(o)) = (s.yards_to_endzone, offense) {
			if let Some(d) = so_far.other(o) {
				line.push(' ');
				line.push_str(&spot(y, o, d));
			}
		}
		Some(line)
	});
	Strip {
		clock: game_clock(at_period, clock, seconds),
		possession: offense.map(|team| [team, " has the ball"].concat()),
		down,
		score: Some(score(game, so_far.score())),
	}
}
