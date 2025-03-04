use scraper::element_ref::ElementRef;
use std::io;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParserError {
	#[error("Failed to parse HTML")]
	HtmlParseError,

	#[error("Missing team name element in the HTML")]
	MissingTeamNameElement,

	#[error("Missing score elements in the HTML for team: {team_name}")]
	MissingScoreElements { team_name: String },

	#[error("Missing date element in the HTML")]
	MissingDateElement,

	#[error("Failed to parse score as a valid number for team: {team_name}, quarter: {quarter}")]
	InvalidScoreFormat {
		team_name: String,
		quarter: usize,
		source: std::num::ParseIntError,
	},

	#[error(transparent)]
	Io(#[from] io::Error),

	#[error(transparent)]
	CsvError(#[from] csv::Error),
}

impl ParserError {
	pub fn missing_team_name_error() -> Self {
		ParserError::MissingTeamNameElement
	}

	pub fn missing_score_elements_error(team_name: String) -> Self {
		ParserError::MissingScoreElements { team_name }
	}

	pub fn missing_date_error() -> Self {
		ParserError::MissingDateElement
	}

	pub fn csv_error(source: csv::Error) -> Self {
		ParserError::CsvError(source)
	}

	pub fn invalid_score_format_error(team_name: String, quarter: usize, source: std::num::ParseIntError) -> Self {
		ParserError::InvalidScoreFormat { team_name, quarter, source }
	}
}
