//! Adapter: an nflverse `games.csv` into [`playoff_delta::season::Season`].
//!
//! `playoff_delta` is pure: it takes a checked season and returns the story.
//! Where the games come from is this module's business, and the only place a
//! feed's spelling (`LA` for the Rams, `game_type`, empty scores for games
//! not played) is known. Another feed gets its own adapter beside this one;
//! the crate does not change.
//!
//! The format is nflverse's `games.csv`: one row per game, with at least the
//! columns `season`, `game_type`, `week`, `away_team`, `away_score`,
//! `home_team` and `home_score`. Only `game_type == "REG"` rows of the asked
//! season are read; a row with both scores empty is a game not yet played.

use playoff_delta::{
	league::Team,
	season::{Game, Score, Season, SeasonError},
};
use std::fmt;

#[derive(Debug)]
pub enum FeedError {
	/// A column the adapter needs is not in the header.
	MissingColumn(&'static str),
	/// A row whose field does not parse; `line` counts from 1, the header included.
	BadField { line: usize, column: &'static str, value: String },
	/// The asked season has no regular-season rows.
	NoSuchSeason(u16),
	/// The rows parse but do not make a season the evaluator can trust.
	Season(SeasonError),
}

impl fmt::Display for FeedError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::MissingColumn(c) => write!(f, "games.csv has no `{c}` column"),
			Self::BadField { line, column, value } => write!(f, "games.csv line {line}: `{column}` is {value:?}"),
			Self::NoSuchSeason(s) => write!(f, "games.csv has no regular-season games for {s}"),
			Self::Season(e) => write!(f, "games.csv: {e}"),
		}
	}
}

impl std::error::Error for FeedError {}

/// nflverse codes that differ from the contract's, including the old homes
/// of relocated teams so a historical season reads too.
fn team(code: &str) -> Option<Team> {
	Team::from_code(match code {
		"LA" | "STL" => "LAR",
		"OAK" => "LV",
		"SD" => "LAC",
		"WSH" => "WAS",
		other => other,
	})
}

/// Splits one CSV record, honouring double quotes (`""` inside a quoted
/// field is a literal quote).
fn fields(line: &str) -> Vec<String> {
	let mut out = Vec::new();
	let mut field = String::new();
	let mut quoted = false;
	let mut chars = line.chars().peekable();
	while let Some(c) = chars.next() {
		match (c, quoted) {
			('"', true) if chars.peek() == Some(&'"') => {
				field.push('"');
				chars.next();
			}
			('"', _) => quoted = !quoted,
			(',', false) => out.push(std::mem::take(&mut field)),
			_ => field.push(c),
		}
	}
	out.push(field);
	out
}

/// The regular season `year` from an nflverse `games.csv`.
///
/// # Errors
/// [`FeedError`]: a missing column, a field that does not parse, no rows for
/// the season, or rows that do not make a valid [`Season`].
pub fn season_from_nflverse(csv: &str, year: u16) -> Result<Season, FeedError> {
	let mut lines = csv.lines();
	let header = fields(lines.next().unwrap_or_default());
	let col = |name: &'static str| header.iter().position(|h| h == name).ok_or(FeedError::MissingColumn(name));
	let (season, kind, week) = (col("season")?, col("game_type")?, col("week")?);
	let (away, away_score, home, home_score) = (col("away_team")?, col("away_score")?, col("home_team")?, col("home_score")?);
	let wanted = year.to_string();
	let mut games = Vec::new();
	for (i, line) in lines.enumerate().filter(|(_, l)| !l.trim().is_empty()) {
		let row = fields(line);
		let get = |c: usize| row.get(c).map_or("", String::as_str);
		if get(season) != wanted || get(kind) != "REG" {
			continue;
		}
		let line = i + 2;
		let bad = |column: &'static str, value: &str| FeedError::BadField {
			line,
			column,
			value: value.to_owned(),
		};
		let parse_team = |c: usize, column: &'static str| team(get(c)).ok_or_else(|| bad(column, get(c)));
		let points = |c: usize, column: &'static str| match get(c) {
			"" | "NA" => Ok(None),
			v => v.parse::<u16>().map(Some).map_err(|_| bad(column, v)),
		};
		let score = match (points(home_score, "home_score")?, points(away_score, "away_score")?) {
			(Some(home), Some(away)) => Some(Score { home, away }),
			(None, None) => None,
			_ => return Err(bad("home_score", get(home_score))),
		};
		games.push(Game {
			week: get(week).parse().map_err(|_| bad("week", get(week)))?,
			home: parse_team(home, "home_team")?,
			away: parse_team(away, "away_team")?,
			score,
			collapse: None,
		});
	}
	if games.is_empty() {
		return Err(FeedError::NoSuchSeason(year));
	}
	Season::new(games).map_err(FeedError::Season)
}

#[cfg(test)]
mod tests {
	use super::*;

	const HEADER: &str = "game_id,season,game_type,week,gameday,away_team,away_score,home_team,home_score,stadium\n";

	/// A full 17-game season, as rows: every team meets 17 others once (the
	/// circle method), with `final_weeks` of it played.
	fn csv(year: u16, final_weeks: u8) -> String {
		let mut out = String::from(HEADER);
		let mut ring: Vec<usize> = (0..32).collect();
		for week in 1..=17u8 {
			for i in 0..16 {
				let (home, away) = (Team::ALL[ring[i]].code(), Team::ALL[ring[31 - i]].code());
				let home = if home == "LAR" { "LA" } else { home };
				let scores = if week <= final_weeks { "24,17" } else { "," };
				let (away_score, home_score) = scores.split_once(',').unwrap_or_default();
				for part in [
					"x",
					&year.to_string(),
					"REG",
					&week.to_string(),
					"2026-09-10",
					away,
					away_score,
					home,
					home_score,
					"\"Stadium, Somewhere\"",
				] {
					out.push_str(part);
					out.push(',');
				}
				out.pop();
				out.push('\n');
			}
			let last = ring.pop().unwrap_or_default();
			ring.insert(1, last);
		}
		out
	}

	#[test]
	fn reads_a_season_in_progress() {
		let text = csv(2026, 5) + &csv(2025, 17).replace(HEADER, "");
		let season = season_from_nflverse(&text, 2026).unwrap();
		assert_eq!(season.length(), 17);
		assert_eq!(season.complete_through(), 5);
		assert!(season.games().iter().any(|g| g.home == Team::Lar || g.away == Team::Lar));
		assert_eq!(season_from_nflverse(&text, 2025).unwrap().complete_through(), 17);
	}

	#[test]
	fn refuses_what_it_cannot_read() {
		assert!(matches!(season_from_nflverse("season,week\n", 2026), Err(FeedError::MissingColumn("game_type"))));
		assert!(matches!(season_from_nflverse(&csv(2026, 1), 2019), Err(FeedError::NoSuchSeason(2019))));
		let typo = csv(2026, 1).replacen(",NE,", ",NWE,", 1);
		assert!(matches!(season_from_nflverse(&typo, 2026), Err(FeedError::BadField { .. })));
		let lopsided = csv(2026, 1).replacen(",24,", ",24x,", 1);
		assert!(matches!(season_from_nflverse(&lopsided, 2026), Err(FeedError::BadField { .. })));
	}
}
