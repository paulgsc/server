//! Adapter: ESPN scoreboard responses into [`playoff_delta::season::Season`].
//!
//! `playoff_delta` is pure: a checked season in, the story out. ESPN's
//! spelling (`WSH` for Washington, scores as strings, `status.type.completed`)
//! is known here and nowhere else.
//!
//! One response per week, each the body of
//! `site.api.espn.com/apis/site/v2/sports/football/nfl/scoreboard?dates=<year>&seasontype=2&week=<n>&limit=40`.
//! Only regular-season events (`season.type == 2`) of the asked year are read;
//! an event not yet final is a game still to play.

use playoff_delta::{
	league::Team,
	season::{Game, Score, Season, SeasonError},
};
use serde_json::Value;
use std::fmt;

#[derive(Debug)]
pub(crate) enum FeedError {
	/// A response is not JSON.
	NotJson(serde_json::Error),
	/// An event lacks a field the adapter needs, or holds one it cannot read.
	BadEvent { event: String, field: &'static str },
	/// No regular-season events for the asked year.
	NoSuchSeason(u16),
	/// The events parse but do not make a season the evaluator can trust.
	Season(SeasonError),
}

impl fmt::Display for FeedError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::NotJson(e) => write!(f, "an ESPN response is not JSON: {e}"),
			Self::BadEvent { event, field } => write!(f, "ESPN event {event}: cannot read `{field}`"),
			Self::NoSuchSeason(y) => write!(f, "no ESPN regular-season events for {y}"),
			Self::Season(e) => write!(f, "ESPN season: {e}"),
		}
	}
}

impl std::error::Error for FeedError {}

/// ESPN codes that differ from the contract's.
fn team(code: &str) -> Option<Team> {
	Team::from_code(match code {
		"WSH" => "WAS",
		"LA" => "LAR",
		other => other,
	})
}

/// One event as a game; `None` for an event outside the asked regular season.
fn game(event: &Value, year: u16) -> Result<Option<Game>, FeedError> {
	let id = event["id"].as_str().unwrap_or("?");
	let bad = |field: &'static str| FeedError::BadEvent { event: id.to_owned(), field };
	if event["season"]["type"].as_u64() != Some(2) || event["season"]["year"].as_u64() != Some(u64::from(year)) {
		return Ok(None);
	}
	let week = event["week"]["number"].as_u64().and_then(|w| u8::try_from(w).ok()).ok_or_else(|| bad("week.number"))?;
	let done = event["status"]["type"]["completed"].as_bool().ok_or_else(|| bad("status.type.completed"))?;
	let competitors = event["competitions"][0]["competitors"].as_array().ok_or_else(|| bad("competitions[0].competitors"))?;
	let side = |home_away: &str| -> Result<(Team, Option<u16>), FeedError> {
		let c = competitors.iter().find(|c| c["homeAway"] == home_away).ok_or_else(|| bad("competitors[].homeAway"))?;
		let t = c["team"]["abbreviation"].as_str().and_then(team).ok_or_else(|| bad("team.abbreviation"))?;
		Ok((t, c["score"].as_str().and_then(|s| s.parse().ok())))
	};
	let ((home, home_score), (away, away_score)) = (side("home")?, side("away")?);
	let score = if done {
		Some(Score {
			home: home_score.ok_or_else(|| bad("score"))?,
			away: away_score.ok_or_else(|| bad("score"))?,
		})
	} else {
		None
	};
	Ok(Some(Game {
		week,
		home,
		away,
		score,
		collapse: None,
		kickoff: event["date"].as_str().and_then(minutes),
	}))
}

/// Minutes since the Unix epoch of an ESPN `date` (`2026-11-01T18:00Z`, UTC,
/// seconds optional); `None` for anything else.
fn minutes(date: &str) -> Option<u64> {
	let num = |range: std::ops::Range<usize>| date.get(range)?.parse::<u64>().ok();
	let (y, m, d, h, min) = (num(0..4)?, num(5..7)?, num(8..10)?, num(11..13)?, num(14..16)?);
	if date.get(4..5) != Some("-") || date.get(10..11) != Some("T") || !(1..=12).contains(&m) || !(1..=31).contains(&d) || h > 23 || min > 59 {
		return None;
	}
	// Days from the civil date (Howard Hinnant's algorithm), March-based years.
	let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
	let era = y / 400;
	let yoe = y % 400;
	let doy = (153 * m + 2) / 5 + d - 1;
	let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
	let days = (era * 146_097 + doe).checked_sub(719_468)?;
	Some((days * 24 + h) * 60 + min)
}

/// The regular season `year` from ESPN scoreboard responses, one per week, in
/// any order.
///
/// # Errors
/// [`FeedError`]: a response that is not JSON, an event missing a field, no
/// events for the season, or events that do not make a valid [`Season`].
pub(crate) fn season_from_scoreboards<'a>(responses: impl IntoIterator<Item = &'a str>, year: u16) -> Result<Season, FeedError> {
	let mut games = Vec::new();
	for body in responses {
		let page: Value = serde_json::from_str(body).map_err(FeedError::NotJson)?;
		for event in page["events"].as_array().map_or(&[][..], Vec::as_slice) {
			games.extend(game(event, year)?);
		}
	}
	if games.is_empty() {
		return Err(FeedError::NoSuchSeason(year));
	}
	Season::new(games).map_err(FeedError::Season)
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	/// One week's scoreboard: the circle method's round `week`, final or not.
	fn scoreboard(year: u16, week: u8, done: bool) -> String {
		let mut ring: Vec<usize> = (0..32).collect();
		for _ in 1..week {
			let last = ring.pop().unwrap();
			ring.insert(1, last);
		}
		let code = |t: Team| if t == Team::Was { "WSH" } else { t.code() };
		let events: Vec<Value> = (0..16)
			.map(|i| {
				let (home, away) = (Team::ALL[ring[i]], Team::ALL[ring[31 - i]]);
				let (home_score, away_score) = if done { ("24", "17") } else { ("0", "0") };
				let date = ["2026-09-10T17:00Z", "2026-09-10T18:00Z", "2026-09-10T19:00Z"][i % 3];
				json!({
					"id": "401",
					"season": { "year": year, "type": 2 },
					"week": { "number": week },
					"date": date,
					"status": { "type": { "completed": done } },
					"competitions": [{ "competitors": [
						{ "homeAway": "home", "team": { "abbreviation": code(home) }, "score": home_score },
						{ "homeAway": "away", "team": { "abbreviation": code(away) }, "score": away_score },
					]}],
				})
			})
			.collect();
		json!({ "events": events }).to_string()
	}

	#[test]
	fn reads_a_season_in_progress() {
		let pages: Vec<String> = (1..=17).map(|w| scoreboard(2026, w, w <= 5)).collect();
		let season = season_from_scoreboards(pages.iter().map(String::as_str), 2026).unwrap();
		assert_eq!((season.length(), season.complete_through()), (17, 5));
		assert!(season.games().iter().any(|g| g.involves(Team::Was)));
	}

	#[test]
	fn refuses_what_it_cannot_read() {
		assert!(matches!(season_from_scoreboards(["{"], 2026), Err(FeedError::NotJson(_))));
		let page = scoreboard(2026, 1, true);
		assert!(matches!(season_from_scoreboards([page.as_str()], 2025), Err(FeedError::NoSuchSeason(2025))));
		let typo = page.replacen("\"NE\"", "\"NWE\"", 1);
		assert!(matches!(season_from_scoreboards([typo.as_str()], 2026), Err(FeedError::BadEvent { .. })));
		let half = scoreboard(2026, 1, true).replacen("\"completed\":true", "\"completed\":false", 1);
		let pages = [half, scoreboard(2026, 2, true)];
		assert!(matches!(season_from_scoreboards(pages.iter().map(String::as_str), 2026), Err(FeedError::Season(_))));
	}

	#[test]
	fn reads_a_kickoff_as_minutes_since_the_epoch() {
		assert_eq!(minutes("1970-01-01T00:00Z"), Some(0));
		assert_eq!(minutes("1970-01-02T01:30Z"), Some(24 * 60 + 90));
		// 2026-11-01T18:00Z is 1,793,556,000 s after the epoch.
		assert_eq!(minutes("2026-11-01T18:00Z"), Some(1_793_556_000 / 60));
		assert_eq!(minutes("2024-02-29T12:00:00Z"), Some(1_709_208_000 / 60));
		for bad in ["", "2026-11-01", "2026/11/01T18:00Z", "2026-13-01T18:00Z", "2026-11-01T24:00Z"] {
			assert_eq!(minutes(bad), None, "{bad}");
		}
		// Each event's `date` reaches its game.
		let page = scoreboard(2026, 1, true);
		let season = season_from_scoreboards([page.as_str()], 2026).unwrap();
		assert!(season.games().iter().all(|g| g.kickoff.is_some_and(|k| k >= minutes("2026-09-10T17:00Z").unwrap())));
	}
}
