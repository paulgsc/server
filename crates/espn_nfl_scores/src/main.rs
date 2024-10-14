mod config;
mod error;

use crate::config::Config;
use crate::error::ParserError;
use clap::Parser;
use csv::Writer;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::{self, Read};
use std::path::Path;

#[derive(Debug)]
struct TeamScore {
	game_id: u32,
	name: String,
	home_away: String,
	quarters: Vec<u32>,
	total: u32,
	date: String,
}

fn read_html_from_file(path: &Path) -> Result<String, io::Error> {
	let mut file = File::open(path)?;
	let mut html = String::new();
	file.read_to_string(&mut html)?;
	Ok(html)
}

fn parse_scores(html: &str) -> Result<Vec<TeamScore>, ParserError> {
	let document = scraper::Html::parse_document(html);

	// Selectors for various elements
	let section_selector = scraper::Selector::parse("section.Card.gameModules").map_err(|_| ParserError::HtmlParseError)?;
	let date_selector = scraper::Selector::parse("header h3.Card__Header__Title").map_err(|_| ParserError::HtmlParseError)?;
	let team_selector = scraper::Selector::parse(".ScoreboardScoreCell__Item").map_err(|_| ParserError::HtmlParseError)?;
	let name_selector = scraper::Selector::parse(".ScoreCell__TeamName").map_err(|_| ParserError::HtmlParseError)?;
	let score_selector = scraper::Selector::parse(".ScoreboardScoreCell__Value").map_err(|_| ParserError::HtmlParseError)?;
	let total_selector = scraper::Selector::parse(".ScoreCell__Score").map_err(|_| ParserError::HtmlParseError)?;

	let mut team_scores = Vec::new();
	let mut game_id = 1;

	// Iterate over each section (each group of games)
	for section in document.select(&section_selector) {
		// Extract the date for the current section
		let date = section
			.select(&date_selector)
			.next()
			.ok_or(ParserError::missing_date_error())?
			.text()
			.collect::<Vec<_>>()
			.join(" ");

		let mut teams_in_game = Vec::new();

		// Iterate over each team in the section
		for team in section.select(&team_selector) {
			let name = team
				.select(&name_selector)
				.next()
				.ok_or(ParserError::missing_team_name_error())?
				.text()
				.collect::<Vec<_>>()
				.join(" ");

			let home_away = team
				.select(&scraper::Selector::parse("span.ScoreboardScoreCell__Record--homeAway").unwrap())
				.next()
				.map_or_else(|| "Unknown".to_string(), |e| e.inner_html());

			let quarters: Vec<u32> = team
				.select(&score_selector)
				.enumerate()
				.map(|(i, score)| {
					score
						.text()
						.collect::<String>()
						.parse::<u32>()
						.map_err(|e| ParserError::invalid_score_format_error(name.clone(), i + 1, e))
				})
				.collect::<Result<Vec<u32>, ParserError>>()?;

			if quarters.is_empty() {
				return Err(ParserError::missing_score_elements_error(name.clone()));
			}

			let total = team
				.select(&total_selector)
				.next()
				.ok_or(ParserError::missing_score_elements_error(name.clone()))?
				.text()
				.collect::<String>()
				.parse::<u32>()
				.map_err(|e| ParserError::invalid_score_format_error(name.clone(), quarters.len(), e))?;

			teams_in_game.push(TeamScore {
				game_id,
				name,
				home_away,
				quarters,
				total,
				date: date.clone(), // Associate the date with the team score
			});

			if teams_in_game.len() == 2 {
				team_scores.extend(teams_in_game.drain(..));
				game_id += 1;
			}
		}
	}

	Ok(team_scores)
}

fn write_to_csv(scores: Vec<TeamScore>, output_path: &Path) -> Result<(), ParserError> {
	let file = OpenOptions::new().append(true).create(true).open(output_path).map_err(ParserError::Io)?;

	let is_empty = file.metadata().map(|m| m.len() == 0).unwrap_or(true);

	let mut wtr = Writer::from_writer(file);

	if is_empty {
		wtr
			.write_record(&["GameID", "Team", "H/A", "Date", "Q1", "Q2", "Q3", "Q4", "OT", "Total"])
			.map_err(ParserError::csv_error)?;
	}

	// Write team data
	for team in scores {
		let mut record = vec![team.game_id.to_string(), team.name, team.home_away, team.date];

		for quarter in team.quarters.iter() {
			record.push(quarter.to_string());
		}
		record.push(team.total.to_string());

		// Fill missing OT value if not present
		if team.quarters.len() < 5 {
			record.insert(8, "0".to_string()); // Assuming OT is 0 if not present
		}

		wtr.write_record(record)?;
	}

	wtr.flush()?;
	Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
	dotenv::dotenv().ok();

	// Load configuration from env.toml
	let config = Config::parse();

	// Read HTML content from the specified input file
	let html = read_html_from_file(Path::new(&config.input_file))?;

	// Parse the HTML content to extract scores
	let scores = parse_scores(&html)?;

	// Write the parsed scores to a CSV file
	write_to_csv(scores, Path::new(&config.output_file))?;

	println!("CSV file generated successfully!");
	Ok(())
}
