//! The week's games, from `/scoreboard`.
//!
//! Home reads it at a slow cadence to learn which games exist this week,
//! when each kicks off, and which are live. The current week is whatever
//! ESPN says it is: the response carries it (`week.number`).

use crate::{int, object, situation::Situation, text, FeedError, Status, Weather};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::Value;

/// One scoreboard response.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scoreboard {
	pub season: Option<i64>,
	/// 1 preseason, 2 regular season, 3 postseason.
	pub season_type: Option<i64>,
	pub week: Option<i64>,
	pub games: Vec<Game>,
	/// Events with no id, which nothing can follow: left out, counted.
	pub skipped: usize,
}

/// One game on the scoreboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
	/// ESPN's event id, which its summary is asked for by.
	pub id: String,
	pub kickoff: Option<DateTime<Utc>>,
	pub status: Status,
	/// `CAR @ GB`.
	pub name: Option<String>,
	pub home: Side,
	pub away: Side,
	pub weather: Option<Weather>,
	/// The snap to come, while the game is live.
	pub situation: Option<Situation>,
}

/// One team in a game.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Side {
	/// ESPN's abbreviation (`WSH`, `LAR`), as ESPN spells it.
	pub team: Option<String>,
	pub score: Option<i64>,
}

/// Parse a scoreboard response.
///
/// # Errors
/// [`FeedError`] when the body is not a JSON object. Anything missing inside
/// one is `None`, or a skipped event when it is the event's id.
pub fn parse(body: &[u8]) -> Result<Scoreboard, FeedError> {
	let page = object(body)?;
	let mut board = Scoreboard {
		season: int(&page["season"]["year"]),
		season_type: int(&page["season"]["type"]),
		week: int(&page["week"]["number"]),
		..Scoreboard::default()
	};
	for event in page["events"].as_array().map_or(&[][..], Vec::as_slice) {
		match game(event) {
			Some(game) => board.games.push(game),
			None => board.skipped += 1,
		}
	}
	Ok(board)
}

fn game(event: &Value) -> Option<Game> {
	let id = text(&event["id"])?;
	let competition = &event["competitions"][0];
	let competitors = competition["competitors"].as_array().map_or(&[][..], Vec::as_slice);
	let side = |home_away: &str| {
		competitors.iter().find(|c| c["homeAway"] == home_away).map_or_else(Side::default, |c| Side {
			team: text(&c["team"]["abbreviation"]),
			score: int(&c["score"]),
		})
	};
	let status = Status::read(if event["status"].is_object() { &event["status"] } else { &competition["status"] });
	Some(Game {
		id,
		kickoff: event["date"].as_str().and_then(kickoff),
		situation: Situation::from_scoreboard(competition, &status),
		status,
		name: text(&event["shortName"]),
		home: side("home"),
		away: side("away"),
		weather: Weather::read(&event["weather"]),
	})
}

/// ESPN writes kickoffs without seconds (`2026-10-30T00:15Z`), which RFC
/// 3339 does not allow; both spellings are read.
#[must_use]
pub fn kickoff(date: &str) -> Option<DateTime<Utc>> {
	DateTime::parse_from_rfc3339(date)
		.map(|at| at.with_timezone(&Utc))
		.ok()
		.or_else(|| NaiveDateTime::parse_from_str(date, "%Y-%m-%dT%H:%MZ").ok().map(|at| at.and_utc()))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::GameState;

	const WEEK8: &[u8] = include_bytes!("../testdata/scoreboard-2026-week8.json");

	#[test]
	fn reads_the_real_week() {
		let board = parse(WEEK8).unwrap();
		assert_eq!((board.season, board.season_type, board.week, board.skipped), (Some(2026), Some(2), Some(8), 0));
		assert_eq!(board.games.len(), 14);
		let first = &board.games[0];
		assert_eq!(first.id, "401873023");
		assert_eq!(first.name.as_deref(), Some("CAR @ GB"));
		assert_eq!(first.kickoff, kickoff("2026-10-30T00:15:00Z"));
		assert_eq!(first.status.state, Some(GameState::Pre));
		assert_eq!(first.status.completed, Some(false));
		assert_eq!(first.home.team.as_deref(), Some("GB"));
		assert_eq!(first.away.team.as_deref(), Some("CAR"));
		let weather = first.weather.as_ref().unwrap();
		assert_eq!((weather.temperature, weather.display.as_deref()), (Some(52), Some("Mostly cloudy")));
		assert!(board.games.iter().all(|g| g.kickoff.is_some() && g.status.state == Some(GameState::Pre)));
	}

	#[test]
	fn a_kickoff_reads_with_or_without_seconds() {
		assert_eq!(kickoff("2026-10-30T00:15Z"), kickoff("2026-10-30T00:15:00Z"));
		assert!(kickoff("2026-10-30T00:15Z").is_some());
		assert_eq!(kickoff("Thursday"), None);
	}

	#[test]
	fn what_is_missing_is_none_and_only_an_unreadable_document_fails() {
		let board = parse(br#"{"events":[{"id":401},{"shortName":"no id"},{"id":"402","status":{"type":{"state":"halftime"}}}]}"#).unwrap();
		assert_eq!(board.skipped, 1);
		assert_eq!(board.games[0].id, "401");
		assert_eq!(board.games[0].status, Status::default());
		assert_eq!(board.games[1].status.state, None);
		assert_eq!(parse(b"{}").unwrap(), Scoreboard::default());
		assert_eq!(parse(b"<html>").unwrap_err().kind(), "not_json");
		assert_eq!(parse(b"[]").unwrap_err().kind(), "not_an_object");
	}
}
