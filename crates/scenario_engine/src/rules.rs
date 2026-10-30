//! The moment rules (SE3): one per catalog [`Moment`], deciding whether a
//! scenario of that kind fits now, and which recent plays were one.
//!
//! Each rule gives a weight, its specificity: a rule that needs more of the
//! moment to line up weighs more, and the offer lists the heaviest first, so
//! a 3rd and 9 in the red zone with a minute left reads Third and long, Red
//! zone, Two-minute drill, Run or throw. Ties keep the catalog's order.

use crate::{
	read::{self, Kick, SoFar, Turnover},
	words, Game, Happened, Mode, Offered, Situation, Watching,
};
use chrono::{DateTime, FixedOffset, Offset, TimeDelta, Utc};
use game_scenarios::{Moment, Scenario};
use nfl_feed::GameState;
use std::cmp::Reverse;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
	Before,
	Live,
	Over,
	/// Over without being played: ESPN says `post` but not completed
	/// (postponed or cancelled), and no play is in the log. Nothing fits.
	NotPlayed,
}

/// Everything a rule reads.
struct Cx<'s> {
	game: &'s Game,
	so_far: &'s SoFar<'s>,
	at: DateTime<Utc>,
	mode: Mode,
	local: FixedOffset,
	watching: Watching,
	phase: Phase,
	/// The side the game is read for.
	us: Option<&'s str>,
}

type Found = Option<(u8, String)>;

impl<'s> Cx<'s> {
	fn new(game: &'s Game, so_far: &'s SoFar<'s>, at: DateTime<Utc>, mode: Mode, local: FixedOffset, watching: Watching) -> Self {
		let phase = if so_far.over() {
			Phase::Over
		} else if so_far.not_played() {
			Phase::NotPlayed
		} else if !so_far.plays.is_empty() || game.status.state == Some(GameState::In) {
			Phase::Live
		} else {
			Phase::Before
		};
		Self {
			game,
			so_far,
			at,
			mode,
			local,
			watching,
			phase,
			us: game.reading.want.as_deref(),
		}
	}

	/// The next snap, while the game is live: the situation and the team with
	/// the ball.
	fn snap_to_come(&self) -> Option<(&'s Situation, &'s str, i64, i64)> {
		if self.phase != Phase::Live {
			return None;
		}
		let situation = self.game.situation.as_ref()?;
		let offense = situation.possession.as_deref()?;
		Some((situation, offense, situation.down?, situation.distance?))
	}

	/// `3rd and 8 at the CHI 35`.
	fn situation_words(&self, situation: &Situation, offense: &str, down: i64, distance: i64) -> String {
		let mut line = words::down_distance(down, distance, situation.yards_to_endzone);
		if let (Some(y), Some(defense)) = (situation.yards_to_endzone, self.so_far.other(offense)) {
			line.push(' ');
			line.push_str(&words::spot(y, offense, defense));
		}
		line
	}

	fn margin_now(&self, team: &str) -> Option<i64> {
		self.so_far.margin(team, self.so_far.score())
	}

	/// The quarter and the seconds left in it, now.
	fn clock(&self) -> (i64, Option<&'s str>, Option<i64>) {
		let latest = self.so_far.plays.last().map(|seen| seen.play);
		let period = self.game.status.period.or_else(|| latest.and_then(|p| p.period)).unwrap_or(1);
		let clock = self.game.status.clock.as_deref().or_else(|| latest.and_then(|p| p.clock.as_deref()));
		(period, clock, clock.and_then(read::clock_seconds))
	}

	/// The team whose points rose on play `i`.
	fn scorer(&self, i: usize) -> Option<&'s str> {
		let (before, after) = (self.so_far.score_before(i), self.so_far.score_after(i));
		if after.1 > before.1 {
			Some(&self.game.home)
		} else if after.0 > before.0 {
			Some(&self.game.away)
		} else {
			None
		}
	}

	/// A touchdown, the latest snap, whose try is not in the log yet.
	fn try_pending(&self) -> Option<&'s str> {
		if self.phase != Phase::Live {
			return None;
		}
		let last = self.so_far.last_snap()?;
		let play = self.so_far.plays[last].play;
		read::try_pending(play).then(|| self.scorer(last).or(play.possession.as_deref())).flatten()
	}

	/// Whether the side we want won: `None` on a tie or with no side.
	fn our_way(&self) -> Option<bool> {
		let margin = self.margin_now(self.us?)?;
		(margin != 0).then_some(margin > 0)
	}

	// What fits now, by moment.

	fn now(&self, moment: Moment, scenario: Scenario) -> Found {
		match moment {
			Moment::Pregame => self.pregame(scenario),
			Moment::Kickoff => self.kickoff(),
			Moment::OpeningDrive => self.opening_drive(),
			Moment::LeadChange => self.lead_change(),
			Moment::ThirdAndLong => self.third_and_long(),
			Moment::FourthDown => self.fourth_down(),
			Moment::Try => self.try_now(),
			Moment::Snap => self.play_call(),
			Moment::RedZone => self.red_zone(),
			Moment::TwoMinute => self.two_minute(),
			Moment::Kick => self.kick_now(),
			Moment::Final => self.final_(scenario),
			Moment::Watching => self.you(scenario),
			// Known only once they have happened: "just happened" offers them.
			Moment::Turnover | Moment::Penalty | Moment::Sack | Moment::BigPlay | Moment::Injury => None,
		}
	}

	/// From the mode's window until the first play.
	fn pregame(&self, scenario: Scenario) -> Found {
		if self.phase != Phase::Before || self.at < self.mode.pregame_opens(self.game.kickoff, self.local) {
			return None;
		}
		let to_kickoff = (self.game.kickoff - self.at).num_seconds();
		match scenario {
			Scenario::Stakes => Some((40, words::stakes(&self.game.reading.team, self.us?))),
			Scenario::Weather => Some((if self.game.weather.is_some() { 35 } else { 15 }, words::forecast(self.game.weather.as_ref()))),
			Scenario::InjuryReport => Some((25, words::before_kickoff(self.game, to_kickoff))),
			Scenario::Venue => Some((20, words::venue(self.game, self.us))),
			_ => None,
		}
	}

	/// The last quarter hour before kickoff, until the first snap.
	fn kickoff(&self) -> Found {
		match self.phase {
			Phase::Before if self.at >= self.game.kickoff - TimeDelta::minutes(15) => Some((55, words::before_kickoff(self.game, (self.game.kickoff - self.at).num_seconds()))),
			Phase::Live if self.so_far.last_snap().is_none() => {
				let kick = self.so_far.plays.iter().find(|seen| read::kind(seen.play) == "kickoff");
				let kicking = kick.and_then(|seen| seen.play.possession.as_deref());
				Some((55, kicking.map_or_else(|| String::from("Kickoff"), |team| words::kicks_off(team, self.so_far.other(team)))))
			}
			_ => None,
		}
	}

	/// The team with the ball is on its first possession of the game.
	fn opening_drive(&self) -> Found {
		let (situation, offense, down, distance) = self.snap_to_come()?;
		self.so_far.first_run_open(offense).then(|| {
			let line = [words::first_drive(offense), String::from(", "), self.situation_words(situation, offense, down, distance)].concat();
			(50, line)
		})
	}

	/// The latest score changed who leads, for the side we want, and the
	/// game has not moved on far from it.
	fn lead_change(&self) -> Found {
		if self.phase != Phase::Live {
			return None;
		}
		let us = self.us?;
		let i = (0..self.so_far.plays.len()).rev().find(|i| self.scorer(*i).is_some())?;
		let (before, after) = (self.so_far.margin(us, self.so_far.score_before(i))?, self.so_far.margin(us, self.so_far.score_after(i))?);
		let swung = (before >= 0 && after < 0) || (before < 0 && after > 0);
		(swung && self.so_far.snaps_since(i) <= 3).then(|| {
			let play = self.so_far.plays[i].play;
			let how = words::score_kind(read::is_touchdown(play), read::field_goal(play).is_some());
			(80, words::lead_change(after, self.scorer(i).unwrap_or(us), how))
		})
	}

	/// 3rd down with 7 or more to go.
	fn third_and_long(&self) -> Found {
		let (situation, offense, down, distance) = self.snap_to_come()?;
		(down == 3 && distance >= 7).then(|| {
			let line = words::at_the_snap(
				offense,
				self.us,
				&words::down_distance(down, distance, situation.yards_to_endzone),
				self.margin_now(offense),
			);
			(85, line)
		})
	}

	fn fourth_down(&self) -> Found {
		let (situation, offense, down, distance) = self.snap_to_come()?;
		(down == 4).then(|| {
			(
				95,
				words::at_the_snap(offense, self.us, &self.situation_words(situation, offense, down, distance), self.margin_now(offense)),
			)
		})
	}

	/// After a touchdown, before the try.
	fn try_now(&self) -> Found {
		let scorer = self.try_pending()?;
		Some((100, words::before_the_try(scorer, self.margin_now(scorer))))
	}

	/// A try, or 4th down in range of a field goal (a 57-yard try or closer).
	fn kick_now(&self) -> Found {
		if let Some(scorer) = self.try_pending() {
			return Some((88, words::kick_or_go(self.margin_now(scorer))));
		}
		let (situation, offense, down, _) = self.snap_to_come()?;
		let to_endzone = situation.yards_to_endzone?;
		(down == 4 && to_endzone <= 40).then(|| (90, words::field_goal_try(offense, self.us, to_endzone + 17, self.margin_now(offense))))
	}

	/// Any snap; a short-yardage or goal-line one calls for a call.
	fn play_call(&self) -> Found {
		if self.try_pending().is_some() {
			return None;
		}
		let (situation, offense, down, distance) = self.snap_to_come()?;
		let short = (down >= 3 && distance <= 2) || situation.yards_to_endzone.is_some_and(|y| y <= 3);
		Some((
			if short { 58 } else { 20 },
			words::at_the_snap(offense, self.us, &self.situation_words(situation, offense, down, distance), None),
		))
	}

	/// Inside the opponent's 20.
	fn red_zone(&self) -> Found {
		let (situation, offense, down, distance) = self.snap_to_come()?;
		situation.yards_to_endzone.filter(|y| *y <= 20).map(|_| {
			(
				70,
				words::at_the_snap(offense, self.us, &self.situation_words(situation, offense, down, distance), self.margin_now(offense)),
			)
		})
	}

	/// The 2nd or 4th quarter with 2:00 or less left.
	fn two_minute(&self) -> Found {
		if self.phase != Phase::Live {
			return None;
		}
		let (period, clock, seconds) = self.clock();
		let seconds = seconds?;
		if !(period == 2 || period == 4) || !(1..=120).contains(&seconds) {
			return None;
		}
		let offense = self.game.situation.as_ref().and_then(|s| s.possession.as_deref());
		let timeouts = offense.map(|team| self.so_far.timeouts_left(team, period));
		Some((60, words::against_the_clock(clock?, offense, self.us, timeouts, period == 4)))
	}

	/// After the final.
	fn final_(&self, scenario: Scenario) -> Found {
		if self.phase != Phase::Over {
			return None;
		}
		let score = self.so_far.score();
		match scenario {
			Scenario::Result => Some((50, words::result_for(self.game, score, &self.game.reading.team, self.us, self.our_way()))),
			Scenario::Paths => Some((40, words::paths(&self.game.reading.team, self.us?, self.our_way()))),
			Scenario::Pattern => Some((30, words::final_line(self.game, score))),
			_ => None,
		}
	}

	/// After the final, from the phone's watching flag.
	fn you(&self, scenario: Scenario) -> Found {
		if self.phase != Phase::Over {
			return None;
		}
		let (us, our_way) = (self.us?, self.our_way()?);
		let score = self.so_far.score();
		let (watched, late) = match self.watching {
			Watching::Unknown => return None,
			Watching::Skipped => (false, None),
			Watching::Watched => (true, None),
			Watching::TunedIn(at) => {
				let then = self.so_far.plays.iter().rposition(|seen| seen.at <= at).map_or((0, 0), |i| self.so_far.score_after(i));
				(true, self.so_far.margin(us, then).filter(|m| *m < 0 && at > self.game.kickoff))
			}
		};
		let fits = match scenario {
			Scenario::CaughtIt => watched && our_way,
			Scenario::SatThrough => watched && !our_way,
			Scenario::MissedIt => !watched && our_way,
			Scenario::DodgedIt => !watched && !our_way,
			Scenario::TunedInLate => late.is_some(),
			_ => false,
		};
		let weight = if scenario == Scenario::TunedInLate { 47 } else { 45 };
		fits.then(|| (weight, words::you(us, watched, late.filter(|_| scenario == Scenario::TunedInLate), our_way, score)))
	}

	// What play `i` was, by moment.

	fn happened(&self, moment: Moment, i: usize) -> Found {
		let play = self.so_far.plays[i].play;
		let snap = read::is_snap(play);
		let offense = play.possession.as_deref();
		let wiped_out = read::text(play).contains("no play");
		let situation = || Some(words::down_distance(play.down?, play.distance?, play.yards_to_endzone));
		match moment {
			Moment::Turnover if !wiped_out => {
				let how = match read::turnover(play)? {
					Turnover::Interception => "threw an interception",
					Turnover::Fumble => "lost a fumble",
					Turnover::OnDowns => "turned it over on downs",
				};
				Some((100, words::turned_over(offense?, how)))
			}
			Moment::LeadChange => {
				let us = self.us?;
				let scorer = self.scorer(i)?;
				let (before, after) = (self.so_far.margin(us, self.so_far.score_before(i))?, self.so_far.margin(us, self.so_far.score_after(i))?);
				((before >= 0 && after < 0) || (before < 0 && after > 0)).then(|| {
					(
						95,
						words::lead_change(after, scorer, words::score_kind(read::is_touchdown(play), read::field_goal(play).is_some())),
					)
				})
			}
			Moment::Try => {
				let team = self.scorer(i).or(offense)?;
				read::is_touchdown(play).then(|| (90, words::touchdown(team, read::try_of(play))))
			}
			Moment::Kick => {
				let team = self.scorer(i).or(offense)?;
				if let Some((kick, yards)) = read::field_goal(play) {
					Some((80, words::kicked(team, "field goal", yards, kick_words(kick))))
				} else {
					read::extra_point_missed(play).map(|kick| (80, words::kicked(team, "extra point", None, kick_words(kick))))
				}
			}
			Moment::FourthDown if snap && !wiped_out && play.down == Some(4) => {
				let result = if read::is_punt(play) {
					"punted"
				} else if read::field_goal(play).is_some() {
					"field goal"
				} else {
					"went for it"
				};
				Some((75, words::down_result(&situation()?, Some(result))))
			}
			Moment::Sack => read::is_sack(play).then(|| (70, words::sacked(offense.unwrap_or("The quarterback"), situation().as_deref()))),
			Moment::BigPlay if snap && !read::is_punt(play) && read::field_goal(play).is_none() => {
				let yards = read::gain(play).filter(|g| *g >= 20)?;
				Some((65, words::big_play(offense?, yards, read::is_touchdown(play))))
			}
			Moment::ThirdAndLong if snap && !wiped_out && play.down == Some(3) && play.distance.is_some_and(|d| d >= 7) => {
				let distance = play.distance?;
				let converted = read::is_touchdown(play) || read::gain(play).is_some_and(|g| g >= distance);
				Some((60, words::down_result(&situation()?, Some(if converted { "converted" } else { "stopped" }))))
			}
			Moment::RedZone if snap && play.yards_to_endzone.is_some_and(|y| y <= 20) => {
				let result = if read::is_touchdown(play) {
					"touchdown"
				} else if read::field_goal(play).is_some() {
					"field goal"
				} else if !wiped_out && read::turnover(play).is_some() {
					"empty"
				} else {
					return None;
				};
				Some((55, words::red_zone_trip(offense?, result)))
			}
			Moment::Injury => read::injury(play).map(|hurt| (50, words::hurt(read::injured_team(play).as_deref(), !hurt))),
			Moment::Penalty => read::penalty(play).map(|(team, foul)| (45, words::flag(&team, foul.as_deref()))),
			Moment::OpeningDrive if self.so_far.in_first_run(i) => {
				let result = if read::is_touchdown(play) {
					"touchdown"
				} else if read::field_goal(play).is_some() {
					"field goal"
				} else if read::is_punt(play) {
					"punted"
				} else if read::turnover(play).is_some() {
					"turned it over"
				} else {
					return None;
				};
				Some((40, words::first_drive_ended(offense?, result)))
			}
			Moment::Kickoff if self.so_far.plays.iter().position(|seen| read::kind(seen.play) == "kickoff") == Some(i) => {
				let kicking = offense?;
				Some((35, words::kicks_off(kicking, self.so_far.other(kicking))))
			}
			Moment::Snap if snap && !wiped_out && (read::is_run(play) || read::is_pass(play)) => {
				let short = (play.down.is_some_and(|d| d >= 3) && play.distance.is_some_and(|d| d <= 2)) || play.yards_to_endzone.is_some_and(|y| y <= 3);
				short.then(|| (30, words::called(read::is_run(play), &situation().unwrap_or_default())))
			}
			_ => None,
		}
	}
}

const fn kick_words(kick: Kick) -> &'static str {
	match kick {
		Kick::Made => "good",
		Kick::Missed => "no good",
		Kick::Blocked => "blocked",
	}
}

/// Heaviest first. `found` is in the catalog's order (one entry per
/// scenario, from [`Scenario::ALL`]) and the sort is stable, so ties keep it.
fn ranked(mut found: Vec<(u8, Scenario, String)>) -> Vec<(Scenario, String)> {
	found.sort_by_key(|f| Reverse(f.0));
	found.into_iter().map(|(_, scenario, line)| (scenario, line)).collect()
}

/// The scenarios that fit now, most specific first.
pub(crate) fn now(game: &Game, so_far: &SoFar<'_>, at: DateTime<Utc>, mode: Mode, local: FixedOffset, watching: Watching) -> Vec<Offered> {
	let cx = Cx::new(game, so_far, at, mode, local, watching);
	let found = Scenario::ALL
		.into_iter()
		.filter_map(|scenario| cx.now(scenario.entry().moment, scenario).map(|(weight, line)| (weight, scenario, line)))
		.collect();
	ranked(found).into_iter().map(|(scenario, line)| Offered { scenario, line }).collect()
}

/// The plays home first saw within the mode's window, newest first, each as
/// the scenarios it was, most specific first.
pub(crate) fn just_happened(game: &Game, so_far: &SoFar<'_>, at: DateTime<Utc>, mode: Mode) -> Vec<Happened> {
	let cx = Cx::new(game, so_far, at, mode, Utc.fix(), Watching::Unknown);
	let since = at - mode.kept();
	let mut out = Vec::new();
	for (i, seen) in so_far.plays.iter().enumerate().rev().filter(|(_, seen)| seen.first >= since) {
		let found = Scenario::ALL
			.into_iter()
			.filter_map(|scenario| cx.happened(scenario.entry().moment, i).map(|(weight, line)| (weight, scenario, line)))
			.collect();
		let ago_seconds = (at - seen.at).num_seconds().max(0);
		out.extend(ranked(found).into_iter().map(|(scenario, line)| Happened {
			scenario,
			play_id: seen.play.id.clone(),
			line,
			ago_seconds,
			ago: words::ago(ago_seconds),
		}));
	}
	out
}
