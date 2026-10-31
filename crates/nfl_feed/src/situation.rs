//! The snap to come, while a game is live: the down, the distance, the spot,
//! the team with the ball and the timeouts each side has left
//! (paulgsc/server#570, for GWB4).
//!
//! The play log holds each play as it stood at its own snap, so the snap
//! about to happen is never in it. ESPN says where the next snap is in two
//! places, and both are read here:
//!
//! - **The scoreboard** carries it whole, as a live competition's
//!   `situation`: `down`, `distance`, `yardLine`, `possession` (the team's
//!   id), `homeTimeouts`, `awayTimeouts`, `isRedZone`.
//! - **The summary** carries no `situation` object. The same facts are
//!   spread over it: the newest play's `end` (the down, distance and spot it
//!   left), and the header's competitors, each with `possession` (a flag) and
//!   `timeoutsUsed`.
//!
//! Home fetches a live game's summary every 15 s and the scoreboard every
//! 10 min, so the summary is what keeps the situation current; the two
//! readings are held equal on responses recorded at the same moment (the
//! `cfb-*-live` fixtures).
//!
//! How ESPN's fields are read, as the recorded responses show them:
//!
//! - `yardLine` counts 0 to 100 from the home team's goal line (as a play's
//!   `start.yardLine` does), so the yards to the end zone are `100 - yardLine`
//!   for the home team and `yardLine` for the away team. That needs to know
//!   who has the ball; without it they are `None`.
//! - The team with the ball is the one ESPN marks: the scoreboard's
//!   `possession`, the summary's competitor with `possession: true`. Between a
//!   kickoff or a score and the next snap ESPN marks neither, and so does
//!   this: the latest play's `end.team` is not read as possession.
//! - `down` and `distance` are `-1` (sometimes `0`) when no snap from scrimmage
//!   is next, after a score or before a kickoff: no down, and no distance
//!   without a down.
//! - Timeouts left are the scoreboard's `homeTimeouts`/`awayTimeouts`, or
//!   three less the summary's `timeoutsUsed`, which held on every live game
//!   recorded. Overtime gives each team two, and whether `timeoutsUsed`
//!   starts again there is not recorded, so in overtime the summary's are
//!   `None`.
//! - `isRedZone` is not carried: it is the yards to the end zone being 20 or
//!   fewer, which a reader works out from [`Situation::yards_to_endzone`].
//!
//! A game that is not `in` (not started, over) has no situation.

use crate::{int, text, GameState, Status};
use serde_json::Value;

/// Each team's timeouts in a half, and so the most `timeoutsUsed` is read
/// against.
const TIMEOUTS_PER_HALF: i64 = 3;

/// The snap to come, as ESPN has it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Situation {
	/// The abbreviation of the team ESPN marks as having the ball, or `None`
	/// when it marks neither.
	pub possession: Option<String>,
	/// 1 to 4, or `None` when no snap from scrimmage is next.
	pub down: Option<i64>,
	/// Yards to go; only with a down.
	pub distance: Option<i64>,
	/// ESPN's `yardLine`: 0 to 100 from the home team's goal line.
	pub yard_line: Option<i64>,
	/// Yards the team with the ball has to the end zone.
	pub yards_to_endzone: Option<i64>,
	pub home_timeouts: Option<i64>,
	pub away_timeouts: Option<i64>,
}

/// Which side of a game a team is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HomeAway {
	Home,
	Away,
}

/// One team of a competition, as its `competitors` entry has it.
struct Competitor<'a> {
	entry: &'a Value,
	side: HomeAway,
	team: Option<String>,
}

fn competitors(competition: &Value) -> Vec<Competitor<'_>> {
	competition["competitors"]
		.as_array()
		.map_or(&[][..], Vec::as_slice)
		.iter()
		.filter_map(|entry| {
			let side = match entry["homeAway"].as_str() {
				Some("home") => HomeAway::Home,
				Some("away") => HomeAway::Away,
				_ => return None,
			};
			Some(Competitor {
				entry,
				side,
				team: text(&entry["team"]["abbreviation"]),
			})
		})
		.collect()
}

impl Situation {
	/// The one rule both readings end in: ESPN's numbers, cleaned of its
	/// placeholders, and the yards to the end zone from the spot and the side
	/// with the ball.
	fn new(spot: &Value, with_ball: Option<&Competitor<'_>>, timeouts: [Option<i64>; 2]) -> Self {
		let down = int(&spot["down"]).filter(|down| (1..=4).contains(down));
		let yard_line = int(&spot["yardLine"]).filter(|yard| (0..=100).contains(yard));
		let yards_to_endzone = with_ball.zip(yard_line).map(|(team, yard)| match team.side {
			HomeAway::Home => 100 - yard,
			HomeAway::Away => yard,
		});
		let [home_timeouts, away_timeouts] = timeouts.map(|left| left.filter(|left| (0..=TIMEOUTS_PER_HALF).contains(left)));
		Self {
			possession: with_ball.and_then(|team| team.team.clone()),
			down,
			distance: down.and_then(|_| int(&spot["distance"]).filter(|yards| *yards > 0)),
			yard_line,
			yards_to_endzone,
			home_timeouts,
			away_timeouts,
		}
	}

	/// A scoreboard competition's `situation`, while the game is live.
	pub(crate) fn from_scoreboard(competition: &Value, status: &Status) -> Option<Self> {
		let situation = &competition["situation"];
		if status.state != Some(GameState::In) || !situation.is_object() {
			return None;
		}
		let teams = competitors(competition);
		let with_ball = text(&situation["possession"]).and_then(|id| teams.iter().find(|team| text(&team.entry["id"]).as_ref() == Some(&id)));
		Some(Self::new(situation, with_ball, [int(&situation["homeTimeouts"]), int(&situation["awayTimeouts"])]))
	}

	/// A summary's situation, while the game is live: the `end` of its newest
	/// play, and its header competition's marks.
	pub(crate) fn from_summary(competition: &Value, status: &Status, newest_play: Option<&Value>) -> Option<Self> {
		if status.state != Some(GameState::In) {
			return None;
		}
		let teams = competitors(competition);
		let with_ball = teams.iter().find(|team| team.entry["possession"].as_bool() == Some(true));
		let overtime = status.period.is_some_and(|period| period > 4);
		let left = |side| {
			if overtime {
				return None;
			}
			let team = teams.iter().find(|team| team.side == side)?;
			int(&team.entry["timeoutsUsed"]).map(|used| TIMEOUTS_PER_HALF - used)
		};
		let spot = newest_play.map_or(&Value::Null, |play| &play["end"]);
		Some(Self::new(spot, with_ball, [left(HomeAway::Home), left(HomeAway::Away)]))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{scoreboard, summary};
	use serde_json::json;

	const BOARD: &[u8] = include_bytes!("../testdata/cfb-scoreboard-2026-10-31-live.json");
	const AWAY_HAS_IT: &[u8] = include_bytes!("../testdata/cfb-summary-401856734-live.json");
	const HOME_HAS_IT: &[u8] = include_bytes!("../testdata/cfb-summary-401858284-live.json");
	const NEITHER_MARKED: &[u8] = include_bytes!("../testdata/cfb-summary-401858508-live.json");
	const NFL_FINAL: &[u8] = include_bytes!("../testdata/summary-401873010-final.json");

	fn on_the_board(game: &str) -> Option<Situation> {
		let board = scoreboard::parse(BOARD).unwrap();
		board.games.into_iter().find(|g| g.id == game).unwrap().situation
	}

	#[test]
	fn the_summary_says_what_the_scoreboard_says_at_the_same_moment() {
		// Recorded together on 2026-10-31, each pair at the same last play.
		let fla = Situation {
			possession: Some("FLA".to_owned()),
			down: Some(2),
			distance: Some(10),
			yard_line: Some(92),
			yards_to_endzone: Some(92),
			home_timeouts: Some(2),
			away_timeouts: Some(2),
		};
		assert_eq!(on_the_board("401856734").as_ref(), Some(&fla), "2nd and 10 at FLA 8, FLA (away) with the ball");
		assert_eq!(summary::parse(AWAY_HAS_IT).unwrap().situation, Some(fla));

		let wake = Situation {
			possession: Some("WAKE".to_owned()),
			down: Some(1),
			distance: Some(10),
			yard_line: Some(60),
			yards_to_endzone: Some(40),
			home_timeouts: Some(2),
			away_timeouts: Some(3),
		};
		assert_eq!(on_the_board("401858284").as_ref(), Some(&wake), "1st and 10 at UVA 40, WAKE (home) with the ball");
		assert_eq!(summary::parse(HOME_HAS_IT).unwrap().situation, Some(wake));

		// Just after a kickoff ESPN marks no team, though the play's own end
		// names the receiving one: neither reading guesses.
		let after_kickoff = Situation {
			possession: None,
			down: Some(1),
			distance: Some(10),
			yard_line: Some(75),
			yards_to_endzone: None,
			home_timeouts: Some(3),
			away_timeouts: Some(2),
		};
		assert_eq!(on_the_board("401858508").as_ref(), Some(&after_kickoff));
		assert_eq!(summary::parse(NEITHER_MARKED).unwrap().situation, Some(after_kickoff));
	}

	#[test]
	fn a_game_not_being_played_has_none() {
		assert_eq!(summary::parse(NFL_FINAL).unwrap().situation, None, "over");
		let pre = include_bytes!("../testdata/summary-401873023-pre.json");
		assert_eq!(summary::parse(pre).unwrap().situation, None, "not started");
		let week8 = scoreboard::parse(include_bytes!("../testdata/scoreboard-2026-week8.json")).unwrap();
		assert!(week8.games.iter().all(|g| g.situation.is_none()));
	}

	/// The recorded NFL game, as its summary would have read live just after
	/// `last`: the plays up to it, `in`, the team with the next snap marked,
	/// and `used` timeouts each (home, then away).
	fn nfl_live_after(last: &summary::Play, marked: &str, used: [i64; 2]) -> Vec<u8> {
		let upto = last.sequence.unwrap();
		let mut page: Value = serde_json::from_slice(NFL_FINAL).unwrap();
		for drive in page["drives"]["previous"].as_array_mut().unwrap() {
			drive["plays"]
				.as_array_mut()
				.unwrap()
				.retain(|play| int(&play["sequenceNumber"]).is_some_and(|at| at <= upto));
		}
		let competition = &mut page["header"]["competitions"][0];
		competition["status"]["type"]["state"] = json!("in");
		competition["status"]["type"]["completed"] = json!(false);
		for competitor in competition["competitors"].as_array_mut().unwrap() {
			let side = usize::from(competitor["homeAway"] == "away");
			competitor["possession"] = json!(competitor["team"]["abbreviation"] == marked);
			competitor["timeoutsUsed"] = json!(used[side]);
		}
		page.to_string().into_bytes()
	}

	#[test]
	fn an_nfl_summary_live_gives_the_next_snap() {
		let plays = summary::parse(NFL_FINAL).unwrap().plays;
		// The 1st-quarter penalty that moved CHI from 3rd and 7 to 3rd and 2
		// at its own 29; the next snap is that 3rd and 2.
		let cut = plays.iter().position(|p| p.text.as_deref().is_some_and(|t| t.contains("Encroachment"))).unwrap();
		let next = &plays[cut + 1];
		assert_eq!(
			(next.possession.as_deref(), next.down, next.distance, next.yards_to_endzone),
			(Some("CHI"), Some(3), Some(2), Some(71))
		);
		// CHI is home; NE has used one timeout.
		let live = summary::parse(&nfl_live_after(&plays[cut], "CHI", [0, 1])).unwrap();
		assert_eq!(live.plays.len(), cut + 1);
		assert_eq!(
			live.situation,
			Some(Situation {
				possession: Some("CHI".to_owned()),
				down: Some(3),
				distance: Some(2),
				yard_line: Some(29),
				yards_to_endzone: Some(71),
				home_timeouts: Some(3),
				away_timeouts: Some(2),
			})
		);
	}

	#[test]
	fn placeholders_and_overtime_read_as_none() {
		let board = json!({ "events": [{ "id": "1", "status": { "type": { "state": "in" }, "period": 2 }, "competitions": [{
			"competitors": [{ "id": "3", "homeAway": "home", "team": { "abbreviation": "CHI" } }],
			"situation": { "down": -1, "distance": -1, "yardLine": 65, "homeTimeouts": 2, "awayTimeouts": 9 },
		}]}]});
		let situation = scoreboard::parse(board.to_string().as_bytes()).unwrap().games[0].situation.clone().unwrap();
		assert_eq!(
			situation,
			Situation {
				yard_line: Some(65),
				home_timeouts: Some(2),
				..Situation::default()
			},
			"after a score: no down, no distance, no team marked, and 9 timeouts is no count"
		);

		let overtime = json!({
			"header": { "competitions": [{ "status": { "type": { "state": "in" }, "period": 5 }, "competitors": [
				{ "id": "3", "homeAway": "home", "team": { "abbreviation": "CHI" }, "possession": true, "timeoutsUsed": 1 },
				{ "id": "17", "homeAway": "away", "team": { "abbreviation": "NE" }, "possession": false, "timeoutsUsed": 0 },
			]}]},
			"drives": { "current": { "plays": [{ "id": "9", "sequenceNumber": "1", "end": { "down": 1, "distance": 10, "yardLine": 25 } }] } },
		});
		let situation = summary::parse(overtime.to_string().as_bytes()).unwrap().situation.unwrap();
		assert_eq!(
			(
				situation.possession.as_deref(),
				situation.down,
				situation.yards_to_endzone,
				situation.home_timeouts,
				situation.away_timeouts
			),
			(Some("CHI"), Some(1), Some(75), None, None)
		);
	}
}
