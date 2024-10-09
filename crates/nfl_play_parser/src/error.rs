use std::num::ParseIntError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GameClockError {
    #[error("Invalid quarter: {0}")]
    InvalidQuarter(String),

    #[error("Invalid minutes: {0}, must be between 0 and 15")]
    InvalidMinutes(u8),

    #[error("Invalid seconds: {0}, must be between 0 and 59")]
    InvalidSeconds(u8),

    #[error("Failed to parse game clock format: {0}")]
    InvalidFormat(String),

    #[error("Parse error occurred for number: {source}")]
    ParseError {
        #[from]
        source: ParseIntError,
    },

    #[error("IO error occurred")]
    IoError(#[from] std::io::Error),
}

impl GameClockError {
    // Specific error creation helpers
    pub fn invalid_quarter_error(quarter: &str) -> Self {
        GameClockError::InvalidQuarter(quarter.to_string())
    }

    pub fn invalid_minutes_error(minutes: u8) -> Self {
        GameClockError::InvalidMinutes(minutes)
    }

    pub fn invalid_seconds_error(seconds: u8) -> Self {
        GameClockError::InvalidSeconds(seconds)
    }

    pub fn invalid_format_error(input: &str) -> Self {
        GameClockError::InvalidFormat(input.to_string())
    }
}

