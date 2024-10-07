mod config;
mod error;

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use clap::Parser;
use csv::Writer;
use crate::config::{Cli, Config};
use crate::error::ParserError;

#[derive(Debug)]
struct TeamScore {
    name: String,
    quarters: Vec<u32>,
    total: u32,
}

fn read_html_from_file(path: &Path) -> Result<String, io::Error> {
    let mut file = File::open(path)?;
    let mut html = String::new();
    file.read_to_string(&mut html)?;
    Ok(html)
}

fn parse_scores(html: &str) -> Result<Vec<TeamScore>, ParserError> {
    let document = scraper::Html::parse_document(html);
    let team_selector = scraper::Selector::parse(".ScoreboardScoreCell__Item").map_err(|_| ParserError::HtmlParseError)?;
    let name_selector = scraper::Selector::parse(".ScoreCell__TeamName").map_err(|_| ParserError::HtmlParseError)?;
    let score_selector = scraper::Selector::parse(".ScoreboardScoreCell__Value").map_err(|_| ParserError::HtmlParseError)?;
    let total_selector = scraper::Selector::parse(".ScoreCell__Score").map_err(|_| ParserError::HtmlParseError)?;

    let mut team_scores = Vec::new();

    for team in document.select(&team_selector) {
        let name = team
            .select(&name_selector)
            .next()
            .ok_or(ParserError::missing_team_name_error())?
            .text()
            .collect::<Vec<_>>()
            .join(" ");

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

        team_scores.push(TeamScore {
            name,
            quarters,
            total,
        });
    }

    Ok(team_scores)
}

fn write_to_csv(scores: Vec<TeamScore>, output_path: &Path) -> Result<(), ParserError> {
    let mut wtr = Writer::from_path(output_path)?;

    // Write header
    wtr.write_record(&["Team", "Q1", "Q2", "Q3", "Q4", "OT", "Total"])?;

    // Write team data
    for team in scores {
        let mut record = vec![team.name];
        for quarter in team.quarters.iter() {
            record.push(quarter.to_string());
        }
        record.push(team.total.to_string());

        // Fill missing OT value if not present
        if team.quarters.len() < 5 {
            record.insert(5, "0".to_string()); // Assuming OT is 0 if not present
        }

        wtr.write_record(record)?;
    }

    wtr.flush()?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse CLI arguments
    let args = Cli::parse();

    // Load configuration from env.toml
    let config = Config::from_toml(&args.config)?;

    // Read HTML content from the specified input file
    let html = read_html_from_file(Path::new(&config.input_file))?;

    // Parse the HTML content to extract scores
    let scores = parse_scores(&html)?;

    // Write the parsed scores to a CSV file
    write_to_csv(scores, Path::new(&config.output_file))?;

    println!("CSV file generated successfully!");
    Ok(())
}

