//! ESPN's NFL feed, parsed (paulgsc/some-ui#2076, GWB2).
//!
//! Home follows the games of the week through ESPN's unofficial site API,
//! the endpoints `apps/playoff_delta_job` already reads:
//!
//! - [`scoreboard_url`]: the current week's games, each with its id, kickoff
//!   and status ([`scoreboard::parse`]).
//! - [`summary_url`]: one game's drives and plays, its status, and before
//!   kickoff the weather forecast ([`summary::parse`]).
//!
//! What this crate promises:
//!
//! - **No I/O.** The caller fetches and hands over the bytes; the same bytes
//!   always give the same output. That is what lets home replay any game from
//!   the responses it kept (GW10, [`log::PlayLog`]).
//! - **Lenient per field, strict per document.** A response that is not a JSON
//!   object is a [`FeedError`]. Inside one, a field ESPN leaves out or spells
//!   differently is `None`, never a failure of the whole response; a play with
//!   no id, the one thing the log keys on, is skipped and counted.
//! - **ESPN's spelling stays here.** Its states (`pre`, `in`, `post`), its
//!   numbers sometimes sent as strings, its kickoff times without seconds and
//!   its `down: 0` on a kickoff are read here and nowhere else.

pub mod log;
pub mod scoreboard;
pub mod summary;

use serde_json::Value;
use std::fmt;

/// ESPN's site API for the NFL.
pub const BASE_URL: &str = "https://site.api.espn.com/apis/site/v2/sports/football/nfl";

/// The current week's scoreboard.
#[must_use]
pub fn scoreboard_url() -> String {
	String::from(BASE_URL) + "/scoreboard"
}

/// One game's summary: drives, plays and status.
#[must_use]
pub fn summary_url(game_id: &str) -> String {
	String::from(BASE_URL) + "/summary?event=" + game_id
}

/// A response this crate cannot read at all.
#[derive(Debug)]
pub enum FeedError {
	/// The bytes are not JSON.
	NotJson(serde_json::Error),
	/// JSON, but not an object.
	NotAnObject,
}

impl FeedError {
	/// The kind, which is all a log line or a metric carries.
	#[must_use]
	pub const fn kind(&self) -> &'static str {
		match self {
			Self::NotJson(_) => "not_json",
			Self::NotAnObject => "not_an_object",
		}
	}
}

impl fmt::Display for FeedError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::NotJson(e) => write!(f, "an ESPN response is not JSON: {e}"),
			Self::NotAnObject => f.write_str("an ESPN response is not a JSON object"),
		}
	}
}

impl std::error::Error for FeedError {}

/// Where a game is, as ESPN's `status.type.state` says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameState {
	/// Not started.
	Pre,
	/// Being played.
	In,
	/// Over (or, with `completed: false`, postponed or cancelled).
	Post,
}

impl GameState {
	/// ESPN's spelling, or `None` for one it has not used before.
	#[must_use]
	pub fn from_espn(state: &str) -> Option<Self> {
		match state {
			"pre" => Some(Self::Pre),
			"in" => Some(Self::In),
			"post" => Some(Self::Post),
			_ => None,
		}
	}

	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Pre => "pre",
			Self::In => "in",
			Self::Post => "post",
		}
	}
}

/// A game's status, from a scoreboard event's or a summary header's
/// `status`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
	pub state: Option<GameState>,
	/// `status.type.completed`: true once the game is final.
	pub completed: Option<bool>,
	pub period: Option<i64>,
	/// The game clock as ESPN shows it (`12:34`).
	pub clock: Option<String>,
}

impl Status {
	/// Whether the game is over and final: `post` and `completed`. A `post`
	/// game that is not completed was postponed or cancelled, and may come
	/// back.
	#[must_use]
	pub fn is_final(&self) -> bool {
		self.state == Some(GameState::Post) && self.completed == Some(true)
	}

	fn read(status: &Value) -> Self {
		Self {
			state: status["type"]["state"].as_str().and_then(GameState::from_espn),
			completed: status["type"]["completed"].as_bool(),
			period: int(&status["period"]),
			clock: text(&status["displayClock"]),
		}
	}
}

/// The forecast ESPN carries before kickoff. Once a game is over its summary
/// and scoreboard leave it out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Weather {
	/// Degrees Fahrenheit.
	pub temperature: Option<i64>,
	/// ESPN's (`AccuWeather`'s) condition code.
	pub condition_id: Option<String>,
	/// The condition in words (`Mostly cloudy`); scoreboard only.
	pub display: Option<String>,
	pub gust: Option<i64>,
	/// Chance of precipitation, in percent.
	pub precipitation: Option<i64>,
}

impl Weather {
	fn read(weather: &Value) -> Option<Self> {
		weather.is_object().then(|| Self {
			temperature: int(&weather["temperature"]),
			condition_id: text(&weather["conditionId"]),
			display: text(&weather["displayValue"]),
			gust: int(&weather["gust"]),
			precipitation: int(&weather["precipitation"]),
		})
	}
}

/// The body as a JSON object.
fn object(body: &[u8]) -> Result<Value, FeedError> {
	let value: Value = serde_json::from_slice(body).map_err(FeedError::NotJson)?;
	if value.is_object() {
		Ok(value)
	} else {
		Err(FeedError::NotAnObject)
	}
}

/// An integer ESPN sends as a number or as a string of digits.
fn int(value: &Value) -> Option<i64> {
	value.as_i64().or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
}

/// A string ESPN sends as a string or as a number.
fn text(value: &Value) -> Option<String> {
	match value {
		Value::String(s) => Some(s.clone()),
		Value::Number(n) => Some(n.to_string()),
		_ => None,
	}
}
