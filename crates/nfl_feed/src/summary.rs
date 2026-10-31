//! One game's play-by-play, from `/summary?event=<id>`.
//!
//! Plays sit in drives: `drives.previous` (finished drives, oldest first)
//! and, while a game is live, `drives.current`. A play in both is read once,
//! from the later place. Plays come out ordered by ESPN's sequence number,
//! then by id, so the same bytes always give the same list.
//!
//! While the game is live, the newest play's `end` and the header's marks
//! also give the snap to come ([`Situation`]). It is kept beside the plays,
//! never in a [`Play`]: a play is what the log compares, and the snap to come
//! changes with every play.

use crate::{int, object, situation::Situation, text, FeedError, Status, Weather};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

/// One summary response.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
	/// The game it is about (`header.id`).
	pub game_id: Option<String>,
	pub status: Status,
	/// Before kickoff only: a final game's summary carries none.
	pub weather: Option<Weather>,
	pub plays: Vec<Play>,
	/// The snap to come, while the game is live.
	pub situation: Option<Situation>,
	/// Plays with no id, which the log cannot key: left out, counted.
	pub skipped: usize,
}

/// One play, normalized. Every field but the id is what ESPN reported, or
/// `None` when it did not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Play {
	/// ESPN's play id, stable across revisions of the play.
	pub id: String,
	/// ESPN's `sequenceNumber`, which orders a game's plays.
	pub sequence: Option<i64>,
	/// When the play happened, as ESPN reports it (`2026-10-23T00:15:46Z`).
	pub wallclock: Option<String>,
	/// The quarter; 5 is overtime.
	pub period: Option<i64>,
	/// The game clock at the snap (`15:00`).
	pub clock: Option<String>,
	/// The down at the snap; `None` where ESPN says 0 (a kickoff, a timeout).
	pub down: Option<i64>,
	/// Yards to go at the snap.
	pub distance: Option<i64>,
	/// ESPN's `start.yardLine`: 0 to 100 from the home team's goal line.
	pub yard_line: Option<i64>,
	/// Yards the team with the ball has to the end zone at the snap.
	pub yards_to_endzone: Option<i64>,
	/// The abbreviation of the team ESPN says has the ball at the snap (the
	/// kicking team on a kickoff).
	pub possession: Option<String>,
	/// ESPN's play type in words (`Pass Reception`, `Rush`, `Timeout`).
	pub play_type: Option<String>,
	pub scoring: Option<bool>,
	/// The score after the play.
	pub away_score: Option<i64>,
	pub home_score: Option<i64>,
	/// ESPN's description of the play.
	pub text: Option<String>,
}

/// Parse a summary response.
///
/// # Errors
/// [`FeedError`] when the body is not a JSON object. Anything missing inside
/// one is `None`, or a skipped play when it is the play's id.
pub fn parse(body: &[u8]) -> Result<Summary, FeedError> {
	let page = object(body)?;
	let competition = &page["header"]["competitions"][0];
	let mut teams: HashMap<String, String> = HashMap::new();
	for competitor in competition["competitors"].as_array().map_or(&[][..], Vec::as_slice) {
		if let (Some(id), Some(code)) = (text(&competitor["id"]), text(&competitor["team"]["abbreviation"])) {
			teams.insert(id, code);
		}
	}
	let drives = &page["drives"];
	let mut drive_list: Vec<&Value> = drives["previous"].as_array().map(|d| d.iter().collect()).unwrap_or_default();
	if drives["current"].is_object() {
		drive_list.push(&drives["current"]);
	}
	let mut summary = Summary {
		game_id: text(&page["header"]["id"]),
		status: Status::read(&competition["status"]),
		weather: crate::Weather::read(&page["gameInfo"]["weather"]),
		..Summary::default()
	};
	let mut plays: BTreeMap<String, Play> = BTreeMap::new();
	// The newest play as ESPN sent it, by the order the list is sorted in;
	// of a play in two places, the later.
	let mut newest: Option<(Option<i64>, String, &Value)> = None;
	for drive in drive_list {
		let drive_team = text(&drive["team"]["abbreviation"]);
		for raw in drive["plays"].as_array().map_or(&[][..], Vec::as_slice) {
			match read_play(raw, &teams, drive_team.as_ref()) {
				Some(play) => {
					if newest.as_ref().is_none_or(|(sequence, id, _)| (play.sequence, &play.id) >= (*sequence, id)) {
						newest = Some((play.sequence, play.id.clone(), raw));
					}
					plays.insert(play.id.clone(), play);
				}
				None => summary.skipped += 1,
			}
		}
	}
	summary.situation = Situation::from_summary(competition, &summary.status, newest.map(|(_, _, raw)| raw));
	summary.plays = plays.into_values().collect();
	summary.plays.sort_by(|a, b| (a.sequence, &a.id).cmp(&(b.sequence, &b.id)));
	Ok(summary)
}

fn read_play(play: &Value, teams: &HashMap<String, String>, drive_team: Option<&String>) -> Option<Play> {
	let start = &play["start"];
	let possession = text(&start["team"]["id"]).and_then(|id| teams.get(&id).cloned()).or_else(|| drive_team.cloned());
	Some(Play {
		id: text(&play["id"])?,
		sequence: int(&play["sequenceNumber"]),
		wallclock: text(&play["wallclock"]),
		period: int(&play["period"]["number"]),
		clock: text(&play["clock"]["displayValue"]),
		down: int(&start["down"]).filter(|down| *down != 0),
		distance: int(&start["distance"]),
		yard_line: int(&start["yardLine"]),
		yards_to_endzone: int(&start["yardsToEndzone"]),
		possession,
		play_type: text(&play["type"]["text"]),
		scoring: play["scoringPlay"].as_bool(),
		away_score: int(&play["awayScore"]),
		home_score: int(&play["homeScore"]),
		text: text(&play["text"]),
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::GameState;
	use serde_json::json;

	const FINAL: &[u8] = include_bytes!("../testdata/summary-401873010-final.json");
	const PRE: &[u8] = include_bytes!("../testdata/summary-401873023-pre.json");

	#[test]
	fn reads_every_play_of_a_finished_game() {
		let summary = parse(FINAL).unwrap();
		assert_eq!(summary.game_id.as_deref(), Some("401873010"));
		assert_eq!(summary.status.state, Some(GameState::Post));
		assert_eq!(summary.status.completed, Some(true));
		assert_eq!(summary.weather, None, "a final game's summary carries no weather");
		assert_eq!((summary.plays.len(), summary.skipped), (180, 0));
		let kickoff = &summary.plays[0];
		assert_eq!(kickoff.id, "40187301040");
		assert_eq!(kickoff.sequence, Some(4000));
		assert_eq!(kickoff.wallclock.as_deref(), Some("2026-10-23T00:15:46Z"));
		assert_eq!((kickoff.period, kickoff.clock.as_deref()), (Some(1), Some("15:00")));
		assert_eq!(
			(kickoff.down, kickoff.distance, kickoff.yard_line, kickoff.yards_to_endzone),
			(None, Some(0), Some(35), Some(65))
		);
		assert_eq!(kickoff.possession.as_deref(), Some("CHI"), "the kicking team has the ball at a kickoff");
		assert_eq!(kickoff.play_type.as_deref(), Some("Kickoff"));
		assert_eq!((kickoff.scoring, kickoff.away_score, kickoff.home_score), (Some(false), Some(0), Some(0)));
		assert!(kickoff.text.as_deref().unwrap().starts_with("C.Santos kicks 60 yards"));
		let snap = &summary.plays[1];
		assert_eq!((snap.down, snap.distance, snap.possession.as_deref()), (Some(1), Some(10), Some("NE")));
		assert!(summary.plays.windows(2).all(|w| w[0].sequence < w[1].sequence));
		assert_eq!(summary.plays.iter().filter(|p| p.scoring == Some(true)).count(), 10);
	}

	#[test]
	fn reads_a_game_before_kickoff() {
		let summary = parse(PRE).unwrap();
		assert_eq!(summary.game_id.as_deref(), Some("401873023"));
		assert_eq!(summary.status.state, Some(GameState::Pre));
		assert!(summary.plays.is_empty());
		let weather = summary.weather.unwrap();
		assert_eq!(
			(weather.temperature, weather.condition_id.as_deref(), weather.gust, weather.precipitation),
			(Some(52), Some("38"), Some(13), Some(13))
		);
	}

	#[test]
	fn a_live_drive_is_read_and_a_play_in_two_drives_once() {
		let mut page: Value = serde_json::from_slice(FINAL).unwrap();
		let mut previous = page["drives"]["previous"].as_array().unwrap().clone();
		let current = previous.pop().unwrap();
		// The current drive's first play is also still listed in the last
		// finished drive, revised.
		let mut repeated = current["plays"][0].clone();
		repeated["text"] = json!("an earlier telling");
		previous.last_mut().unwrap()["plays"].as_array_mut().unwrap().push(repeated);
		page["drives"] = json!({ "previous": previous, "current": current });
		let live = parse(page.to_string().as_bytes()).unwrap();
		assert_eq!(live.plays, parse(FINAL).unwrap().plays);
	}

	#[test]
	fn a_play_missing_fields_is_kept_and_one_missing_its_id_is_skipped() {
		let body = json!({ "drives": { "previous": [{ "plays": [
			{ "id": 7, "sequenceNumber": "12" },
			{ "text": "no id" },
		]}]}});
		let summary = parse(body.to_string().as_bytes()).unwrap();
		assert_eq!(summary.skipped, 1);
		let play = &summary.plays[0];
		assert_eq!((play.id.as_str(), play.sequence), ("7", Some(12)));
		assert_eq!((play.possession.as_ref(), play.text.as_ref(), play.scoring), (None, None, None));
		assert_eq!(parse(b"{}").unwrap(), Summary::default());
		assert_eq!(parse(b"Too Many Requests").unwrap_err().kind(), "not_json");
	}
}
