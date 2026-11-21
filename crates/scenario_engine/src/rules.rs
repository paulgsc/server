//! The moment rules (SE3): one per catalog [`Moment`], deciding whether a
//! scenario of that kind fits now, and which recent plays were one.
//!
//! Each rule gives a weight, its specificity: a rule that needs more of the
//! moment to line up weighs more, and the offer lists the heaviest first, so
//! a 3rd and 9 in the red zone with a minute left reads Third and long, Red
//! zone, Two-minute drill, Run or throw. Ties keep the catalog's order.

use crate::{
	read::{self, Kick, SoFar, Turnover},
	words, BigPlay, Game, Happened, Mode, Offered, Ranked, Said, Situation, Tier, Watching,
};
use chrono::{DateTime, FixedOffset, Offset, TimeDelta, Utc};
use game_scenarios::{Kind, Moment, Phase as When, Scenario, Unit};
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
			Moment::Halftime => self.halftime(),
			// Known only once they have happened: "just happened" offers them.
			Moment::Turnover | Moment::Penalty | Moment::Sack | Moment::BigPlay | Moment::Injury | Moment::OnsideKick => None,
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

	/// The last quarter hour before kickoff, until the opening kickoff. Once
	/// the kickoff is in the log it is in "just happened" instead, so the
	/// first snap's own moments (the opening drive) lead.
	fn kickoff(&self) -> Found {
		match self.phase {
			Phase::Before if self.at >= self.game.kickoff - TimeDelta::minutes(15) => Some((55, words::before_kickoff(self.game, (self.game.kickoff - self.at).num_seconds()))),
			Phase::Live if self.so_far.last_snap().is_none() && self.so_far.opening_kickoff().is_none() => Some((55, String::from("Kickoff"))),
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

	/// From ESPN's end of the half until the second half's kickoff.
	fn halftime(&self) -> Found {
		if self.phase != Phase::Live {
			return None;
		}
		let plays = &self.so_far.plays;
		let end = plays.iter().rposition(|seen| read::ends_the_half(seen.play))?;
		if plays[end + 1..].iter().any(|seen| read::kind(seen.play).contains("kickoff") || read::is_snap(seen.play)) {
			return None;
		}
		let score = words::score(self.game, self.so_far.score());
		Some((100, words::halftime(self.us, self.us.and_then(|us| self.margin_now(us)), &score)))
	}

	/// Play `i` by the big-play algebra: a gain of 20 or more, credited to the
	/// side that gained it.
	fn big_play(&self, i: usize) -> Option<BigPlay> {
		let play = self.so_far.plays[i].play;
		let (kind, yards, theirs) = read::gain_by(play)?;
		let size = kind.size(yards)?;
		let team = play.possession.as_deref()?;
		let by = if theirs { team } else { self.so_far.other(team)? };
		Some(BigPlay {
			size,
			kind,
			by: by.to_owned(),
			yards,
			scored: read::is_touchdown(play),
			good: self.us.map(|us| us == by),
		})
	}

	/// Offense or defense as the owner says it: with a side wanted, its
	/// offense when `team` had the ball and its defense when the other team
	/// did; with none, the offense of `team`.
	fn by_the_ball(&self, team: Option<&str>) -> Option<(Unit, Option<String>)> {
		let team = team?;
		Some(match self.us {
			Some(us) if us != team => (Unit::Defense, Some(us.to_owned())),
			_ => (Unit::Offense, Some(team.to_owned())),
		})
	}

	/// Whether a bite would be about a unit at all now: only in the game's
	/// own scenarios while it is played. Before kickoff (ESPN may say the game
	/// is on before the log holds its opening kickoff) and after the final, a
	/// bite is about the game, whatever scenario it names.
	fn about_a_unit(&self, scenario: Scenario) -> bool {
		scenario.entry().phase == When::In && self.phase == Phase::Live && (self.so_far.last_snap().is_some() || self.so_far.opening_kickoff().is_some())
	}

	/// What a bite of `scenario` is about by default, now (#3049, C874): a
	/// unit, and whose as the team's code. This is the default for a pick
	/// from the list (the cards, and the rest); a play that just happened
	/// carries its own, [`Cx::happened_unit`].
	///
	/// - A call (4th down, the try, the two-minute drill) is the coaching of
	///   the team with the decision; halftime is the side wanted's, or no one
	///   team's.
	/// - A kick is the special teams of the team kicking.
	/// - What the moment's rule offers ahead (a 3rd and long, a snap, the red
	///   zone, the opening drive) goes by the ball as [`Cx::by_the_ball`]
	///   says, the snap to come's.
	/// - What only ever just happened (a turnover, a flag, a sack, an injury,
	///   a lead change, a big play, an onside kick) is the latest such play's
	///   own default; with none yet, it goes by the latest play's ball.
	///
	/// None before kickoff, after the final, or for a pregame, final or
	/// owner's scenario; none too when the team it would name is unknown.
	fn default_unit(&self, scenario: Scenario) -> Option<(Unit, Option<String>)> {
		if !self.about_a_unit(scenario) {
			return None;
		}
		let moment = scenario.entry().moment;
		let to_come = self.game.situation.as_ref().and_then(|s| s.possession.as_deref());
		let last = self.so_far.plays.iter().rev().find_map(|seen| seen.play.possession.as_deref());
		let latest = || (0..self.so_far.plays.len()).rev().find(|&i| self.happened(moment, i).is_some());
		match moment {
			Moment::FourthDown | Moment::TwoMinute => Some((Unit::Coaching, Some(to_come.or(last)?.to_owned()))),
			Moment::Try => Some((Unit::Coaching, Some(self.try_pending().or(last)?.to_owned()))),
			Moment::Halftime => Some((Unit::Coaching, self.us.map(str::to_owned))),
			// The try's kick is the scorer's; a field goal try, the offense's.
			Moment::Kick => Some((Unit::SpecialTeams, Some(self.try_pending().or(to_come).or(last)?.to_owned()))),
			// The team that kicks off next: the latest onside kick's only when
			// nothing has scored since, else the team that scored last.
			Moment::OnsideKick => {
				let scored = (0..self.so_far.plays.len()).rev().find(|&i| self.scorer(i).is_some());
				match (latest(), scored) {
					(Some(onside), scored) if scored.is_none_or(|scored| onside > scored) => self.happened_unit(moment, onside),
					(_, scored) => Some((Unit::SpecialTeams, Some(self.scorer(scored?)?.to_owned()))),
				}
			}
			Moment::Turnover | Moment::Penalty | Moment::Sack | Moment::Injury | Moment::LeadChange | Moment::BigPlay => {
				latest().map_or_else(|| self.by_the_ball(last), |i| self.happened_unit(moment, i))
			}
			Moment::ThirdAndLong | Moment::Snap | Moment::RedZone | Moment::OpeningDrive => self.by_the_ball(to_come.or(last)),
			// The coin toss: before the opening kickoff, no one team's yet.
			Moment::Kickoff | Moment::Pregame | Moment::Final | Moment::Watching => None,
		}
	}

	/// What a bite of play `i`, just happened as `moment`, is about by
	/// default (#3049, C874): read off that play, not off the snap to come.
	/// A call is the coaching of the team that made it; a kick, the kicking
	/// team's special teams (the try's, the scorer's); a return, the
	/// returners'; a score, the scorer's (its defense or special teams when
	/// the other team had the ball); anything else goes by the play's ball.
	fn happened_unit(&self, moment: Moment, i: usize) -> Option<(Unit, Option<String>)> {
		if self.phase != Phase::Live {
			return None;
		}
		let play = self.so_far.plays[i].play;
		let offense = play.possession.as_deref();
		let of = |unit, team: Option<&str>| Some((unit, Some(team?.to_owned())));
		match moment {
			Moment::FourthDown => of(Unit::Coaching, offense),
			Moment::Try => of(Unit::Coaching, self.scorer(i).or(offense)),
			Moment::Kick => of(Unit::SpecialTeams, self.scorer(i).or(offense)),
			Moment::OnsideKick | Moment::Kickoff => of(Unit::SpecialTeams, offense),
			Moment::BigPlay => self.big_play(i).and_then(|big| self.gained(big.kind, &big.by)),
			Moment::LeadChange => {
				let scorer = self.scorer(i)?;
				match read::gain_by(play) {
					// A return for the score: the returners'.
					Some((kind, _, false)) => self.gained(kind, scorer),
					_ => self.by_the_ball(Some(scorer)),
				}
			}
			Moment::Turnover | Moment::Penalty | Moment::Sack | Moment::Injury | Moment::ThirdAndLong | Moment::Snap | Moment::RedZone | Moment::OpeningDrive => {
				self.by_the_ball(offense)
			}
			Moment::TwoMinute | Moment::Halftime | Moment::Pregame | Moment::Final | Moment::Watching => None,
		}
	}

	/// Whose unit a gain of `kind` by `by` was: a kick or punt return is the
	/// returners' special teams, a pass or run goes by the ball, and an
	/// interception or fumble return is the defense's (as the owner says it:
	/// ours, or our offense's when the other side returned it).
	fn gained(&self, kind: Kind, by: &str) -> Option<(Unit, Option<String>)> {
		match kind {
			Kind::KickReturn | Kind::PuntReturn => Some((Unit::SpecialTeams, Some(by.to_owned()))),
			Kind::Pass | Kind::Run => self.by_the_ball(Some(by)),
			Kind::InterceptionReturn | Kind::FumbleReturn => Some(match self.us {
				Some(us) if us != by => (Unit::Offense, Some(us.to_owned())),
				_ => (Unit::Defense, Some(by.to_owned())),
			}),
		}
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
		let wiped_out = read::wiped_out(play);
		let situation = || Some(words::down_distance(play.down?, play.distance?, play.yards_to_endzone));
		match moment {
			Moment::Turnover => {
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
			Moment::BigPlay => self.big_play(i).map(|big| (65, words::big_play(&big))),
			Moment::OnsideKick if read::is_onside(play) && !wiped_out => {
				let kicking = offense?;
				// A kick the receivers fielded is theirs; ESPN names a recovery
				// only when it was loose.
				let recovered = read::recovered_by(play).or_else(|| self.so_far.other(kicking).map(str::to_owned));
				Some((85, words::onside(kicking, recovered.as_deref())))
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
				} else if read::turnover(play).is_some() {
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
			Moment::Kickoff if self.so_far.opening_kickoff() == Some(i) => {
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

/// Whether a play can be one of `moment`'s scenarios: the moments with a
/// "just happened" rule in [`Cx::happened`]. The rest (pregame, the two-minute
/// drill, halftime, the final and the owner's own) are about a stretch of the
/// game, not one play.
pub(crate) const fn about_a_play(moment: Moment) -> bool {
	!matches!(moment, Moment::Pregame | Moment::TwoMinute | Moment::Final | Moment::Watching | Moment::Halftime)
}

/// When a bite offered as `scenario` at `at` was said relative to its play:
/// before it when the moment's rule offers the snap to come, after it when
/// the rule offers a play already in the log. One arm per moment, so a new
/// moment must say which (#3040).
pub(crate) fn offered_said(game: &Game, so_far: &SoFar<'_>, at: DateTime<Utc>, scenario: Scenario) -> Said {
	let cx = Cx::new(game, so_far, at, Mode::Full, Utc.fix(), Watching::Unknown);
	match scenario.entry().moment {
		// A try is in its touchdown's own play: that play is in the log.
		Moment::Kick if cx.try_pending().is_some() => Said::After,
		// Offered on the drive the snap to come is in: its play ends it.
		Moment::OpeningDrive | Moment::RedZone => Said::BeforeTheDrive,
		// Offered on the snap to come, which is its play: the down and
		// distance, a play call, a field goal try.
		Moment::ThirdAndLong | Moment::FourthDown | Moment::Snap | Moment::Kick => Said::BeforeTheSnap,
		// Offered up to a quarter hour ahead: its play is the opening kickoff.
		Moment::Kickoff => Said::BeforeTheKickoff,
		// Offered once the touchdown or the score is in the log; never offered
		// ahead of time ("just happened" offers them); or about no one play.
		Moment::Try
		| Moment::LeadChange
		| Moment::Turnover
		| Moment::Penalty
		| Moment::Sack
		| Moment::BigPlay
		| Moment::Injury
		| Moment::Pregame
		| Moment::TwoMinute
		| Moment::Final
		| Moment::Watching
		// Halftime is about no one play; an onside kick is offered only once
		// it is in the log.
		| Moment::Halftime
		| Moment::OnsideKick => Said::After,
	}
}

/// The play of `so_far` a bite of `scenario` said as `said` is about
/// ([`crate::its_play`] says how).
#[allow(clippy::too_many_arguments)] // as `its_play`
pub(crate) fn its_play(
	game: &Game,
	so_far: &SoFar<'_>,
	now: DateTime<Utc>,
	scenario: Scenario,
	said: Said,
	at: DateTime<Utc>,
	offered: DateTime<Utc>,
	window: TimeDelta,
) -> Option<usize> {
	let accepted = were(game, so_far, now, scenario);
	let plays = &so_far.plays;
	let before = || {
		accepted
			.iter()
			.copied()
			.filter(|i| plays[*i].at <= at && at - plays[*i].at <= window)
			.max_by_key(|i| plays[*i].at)
	};
	let to_come = || {
		(0..plays.len()).find(|i| {
			let play = plays[*i].play;
			plays[*i].first > offered && read::is_snap(play) && !read::wiped_out(play)
		})
	};
	match said {
		Said::After => before(),
		// Not tied to an offer: the nearer in time of the nearest accepted
		// play at or before the bite and the first after it, a tie looking
		// back.
		Said::Either => {
			let after = accepted.iter().copied().find(|i| plays[*i].at > at && plays[*i].at - at <= window);
			match (before(), after) {
				(Some(b), Some(a)) => Some(if plays[a].at - at < at - plays[b].at { a } else { b }),
				(b, a) => b.or(a),
			}
		}
		Said::BeforeTheKickoff => so_far.opening_kickoff().filter(|i| plays[*i].first > offered && accepted.contains(i)),
		Said::BeforeTheSnap => {
			let next = to_come().filter(|i| (at - plays[*i].at).abs() <= window)?;
			accepted.contains(&next).then_some(next)
		}
		// The window bounds the snap to come; the drive may end long after.
		Said::BeforeTheDrive => {
			let next = to_come().filter(|i| (at - plays[*i].at).abs() <= window)?;
			let run = so_far.run(next)?;
			accepted.into_iter().find(|i| *i >= next && so_far.run(*i) == Some(run))
		}
	}
}

/// The plays of `so_far`, in ESPN's order, that `scenario`'s rule says were
/// one: the "just happened" rule, with no window and no count.
pub(crate) fn were(game: &Game, so_far: &SoFar<'_>, at: DateTime<Utc>, scenario: Scenario) -> Vec<usize> {
	let cx = Cx::new(game, so_far, at, Mode::Full, Utc.fix(), Watching::Unknown);
	let moment = scenario.entry().moment;
	(0..so_far.plays.len()).filter(|i| cx.happened(moment, *i).is_some()).collect()
}

/// Every scenario that fits at `at` (its rule, untruncated) or that a play
/// home first saw within Full mode's window was, in the catalog's order.
pub(crate) fn could_be(game: &Game, so_far: &SoFar<'_>, at: DateTime<Utc>, local: FixedOffset, watching: Watching) -> Vec<Scenario> {
	let cx = Cx::new(game, so_far, at, Mode::Full, local, watching);
	let since = at - Mode::Full.kept();
	let recent: Vec<usize> = (0..so_far.plays.len()).filter(|i| so_far.plays[*i].first >= since).collect();
	Scenario::ALL
		.into_iter()
		.filter(|scenario| {
			let moment = scenario.entry().moment;
			cx.now(moment, *scenario).is_some() || recent.iter().any(|i| cx.happened(moment, *i).is_some())
		})
		.collect()
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
		out.extend(ranked(found).into_iter().map(|(scenario, line)| {
			let (unit, unit_side) = cx
				.happened_unit(scenario.entry().moment, i)
				.filter(|_| cx.about_a_unit(scenario))
				.map_or((None, None), |(unit, side)| (Some(unit), side));
			Happened {
				scenario,
				play_id: seen.play.id.clone(),
				line,
				ago_seconds,
				ago: words::ago(ago_seconds),
				big_play: (scenario == Scenario::BigPlay).then(|| cx.big_play(i)).flatten(),
				unit,
				unit_side,
			}
		}));
	}
	out
}

/// The scenarios plausible at any time of a game week (#3049, C874): what
/// the game means for the paths, and whether it fits a pattern.
const ALWAYS: [Scenario; 2] = [Scenario::Stakes, Scenario::Pattern];

/// Every scenario of the set once (#3049): the cards on offer now, exactly
/// as `offered` lists them (the mode's count), then the always plausible,
/// then the rest in the set's order. Each with its default unit.
pub(crate) fn every_scenario(game: &Game, so_far: &SoFar<'_>, at: DateTime<Utc>, local: FixedOffset, watching: Watching, offered: &[Offered]) -> Vec<Ranked> {
	let cx = Cx::new(game, so_far, at, Mode::Full, local, watching);
	let mut out: Vec<Ranked> = Vec::with_capacity(Scenario::ALL.len());
	let mut add = |scenario: Scenario, tier: Tier| {
		if !out.iter().any(|r| r.scenario == scenario) {
			let (unit, unit_side) = cx.default_unit(scenario).map_or((None, None), |(unit, side)| (Some(unit), side));
			out.push(Ranked { scenario, tier, unit, unit_side });
		}
	};
	for offered in offered {
		add(offered.scenario, Tier::Now);
	}
	for scenario in ALWAYS {
		add(scenario, Tier::Always);
	}
	for scenario in Scenario::ALL {
		add(scenario, Tier::Rest);
	}
	out
}
