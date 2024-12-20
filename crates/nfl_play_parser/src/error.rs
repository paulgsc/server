use std::num::ParseIntError;
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum GameClockError {
	#[error("Invalid quarter: {quarter}")]
	InvalidQuarter { quarter: String }, // Struct-like variant

	#[error("Invalid minutes: must be between 0 and 15")]
	InvalidMinutes { minutes: u8 }, // Struct-like variant

	#[error("Invalid seconds: {seconds}, must be between 0 and 59")]
	InvalidSeconds { seconds: u8 }, // Struct-like variant

	#[error("Invalid time format: {time}")]
	InvalidTimeFormat { time: String },

	#[error("Failed to parse game clock format: {0}")]
	InvalidFormat(String), // This is still a tuple-like variant

	#[error("Parse error occurred for number: {source}")]
	ParseError {
		#[from]
		source: ParseIntError,
	},

	#[error("IO error occurred: {0}")]
	IoError(String),
}

#[derive(Debug, Error, PartialEq)]
pub enum PlayTypeError {
	#[error("Unable to determine play type from: {input}")]
	UnknownPlayType { input: String },
}

#[derive(Debug, Error, PartialEq)]
pub enum TeamAbbreviationError {
	#[error("Invalid team abbreviation: {0}")]
	InvalidTeamAbbreviation(String),
}

#[derive(Debug, Error, PartialEq)]
pub enum ScoringEventError {
	#[error("Unable to determine scoring event type from: {input}")]
	UnknownScoringEventType { input: String },
	#[error("Unable to determine team from: {input}")]
	UnknownTeam { input: String },
}

#[derive(Debug, Error, PartialEq)]
pub enum YardsError {
	#[error("Invalid yards value: {value}, must be between 0 and 100")]
	InvalidYards { value: u8 },

	#[error("Invalid yards description: {0}")]
	InvalidYardsFormat(String),

	#[error("No yards information found in the play description")]
	NoYardsInfo,
}

#[derive(Debug, Error, PartialEq)]
pub enum DownAndDistanceError {
	#[error("Invalid Down")]
	InvalidDown,
	#[error("Invalid Distance parsed string format")]
	InvalidDownDistanceFormat,
	#[error("Invalid Down distance must be between 1 and 100")]
	InvalidDownDistance,
	#[error(transparent)]
	TeamAbbreviationError(#[from] TeamAbbreviationError),
	#[error("Invalid yard line")]
	InvalidYardLine,
}

#[derive(Debug, Error, PartialEq)]
pub enum PlayByPlayError {
	#[error("Missing Valid Play by Play Description")]
	MissingDescription,
	#[error("Missing Down and Distance")]
	MissingHeadline,
	#[error("Invalid play description format")]
	InvalidFormat,
	#[error(transparent)]
	Yards(YardsError),
	#[error(transparent)]
	GameClock(#[from] GameClockError),
	#[error(transparent)]
	DownAndDistance(#[from] DownAndDistanceError),
	#[error(transparent)]
	PlayType(#[from] PlayTypeError),
	#[error(transparent)]
	ScoringEvent(#[from] ScoringEventError),
	#[error("Missing Drive Number")]
	MissingDriveId,
	#[error("Missing Team Name for play drive")]
	MissingTeamName,
}

impl GameClockError {
	// Specific error creation helpers

	// Correct struct-like variant instantiation
	pub fn invalid_quarter_error(quarter: &str) -> Self {
		GameClockError::InvalidQuarter {
			quarter: quarter.to_string(), // Use struct syntax here
		}
	}

	pub fn invalid_minutes_error(minutes: u8) -> Self {
		GameClockError::InvalidMinutes {
			minutes, // Struct syntax for InvalidMinutes
		}
	}

	pub fn invalid_seconds_error(seconds: u8) -> Self {
		GameClockError::InvalidSeconds {
			seconds, // Struct syntax for InvalidSeconds
		}
	}
	pub fn invalid_time_format_error(time: &str) -> Self {
		GameClockError::InvalidTimeFormat { time: time.to_string() }
	}

	pub fn invalid_format_error(input: &str) -> Self {
		GameClockError::InvalidFormat(input.to_string()) // Tuple-like variant syntax remains the same
	}
}

impl PlayTypeError {
	// Helper function to create UnknownPlayType error
	pub fn unknown_play_type(input: &str) -> Self {
		PlayTypeError::UnknownPlayType { input: input.to_string() }
	}
}
