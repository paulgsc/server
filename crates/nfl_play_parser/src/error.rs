use thiserror::Error;
use std::num::ParseIntError;


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
        GameClockError::InvalidTimeFormat {
            time: time.to_string(),
        }
    }

    pub fn invalid_format_error(input: &str) -> Self {
        GameClockError::InvalidFormat(input.to_string()) // Tuple-like variant syntax remains the same
    }
}
