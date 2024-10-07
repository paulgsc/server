use scraper::{Html, Selector};
use std::error::Error;
use csv::Writer;
use crate::error::ParserError;

#[derive(Debug)]
struct TeamScore {
    name: String,
    quarters: Vec<u32>,
    total: u32,
}

fn parse_scores(html: &str) -> Result<Vec<TeamScore>, ParserError> {
    let document = Html::parse_document(html);
    let team_selector = Selector::parse(".ScoreboardScoreCell__Item").map_err(|_| ParserError::HtmlParseError)?;
    let name_selector = Selector::parse(".ScoreCell__TeamName").map_err(|_| ParserError::HtmlParseError)?;
    let score_selector = Selector::parse(".ScoreboardScoreCell__Value").map_err(|_| ParserError::HtmlParseError)?;
    let total_selector = Selector::parse(".ScoreCell__Score").map_err(|_| ParserError::HtmlParseError)?;

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

fn write_to_csv(scores: Vec<TeamScore>) -> Result<(), ParserError> {
    let mut wtr = Writer::from_path("team_scores.csv")?;

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

fn main() -> Result<(), Box<dyn Error>> {
    let html = r#"
    <!-- Your HTML snippet here -->
    "#;

    let scores = parse_scores(html)?;

    // Write the parsed data to CSV
    write_to_csv(scores)?;

    println!("CSV file generated successfully!");
    Ok(())
}

