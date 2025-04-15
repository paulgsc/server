use clap::Parser;
use serde::{Deserialize, Serialize};

#[derive(Parser, Clone, Debug, Serialize, Deserialize)]
#[command(author, version, about, long_about = None)]
pub struct Config {
	/// File with HTML soup
	#[arg(long, env = "HTML_FILE")]
	pub input_file: String,
	#[arg(long, env = "SPREADSHEETID")]
	pub spreadsheet_id: String,
	#[arg(long, env = "SHEETNAME")]
	pub sheet_name: String,
	/// Path to the output CSV file
	#[arg(long, env = "CSV_OUTPUT", default_value = "data.csv", value_parser = validate_output)]
	pub output_file: String,
	/// Get HTML Soup From Gdrive (cloud) or From Local file (local)
	#[arg(long, env = "MODE", default_value = "cloud", value_parser = validate_output)]
	pub mode: String,
}

impl Config {
	#[allow(dead_code)]
	pub fn new() -> Self {
		Self::parse()
	}
}

fn validate_output(value: &str) -> Result<String, String> {
	match value {
		"data.csv" | "gsheet" => Ok(value.to_string()),
		_ => Err("Output file must be either 'data.csv' or 'gsheet'".to_string()),
	}
}
