use scraper::{Html, Selector};
use std::fmt;

pub struct NflScoringSummarySelectors {
	pub quarter_section: Selector,
	pub scoring_section: Selector,
	pub quarter: Selector,
	pub team_logo: Selector,
	pub score_event: Selector,
	pub time: Selector,
	pub score_desc: Selector,
}

impl NflScoringSummarySelectors {
	pub fn new() -> Self {
		NflScoringSummarySelectors {
			quarter_section: Selector::parse(".playByPlay__table--summary").unwrap(),
			scoring_section: Selector::parse(".playByPlay__tableRow").unwrap(),
			quarter: Selector::parse(".playByPlay__quarter h4").unwrap(),
			team_logo: Selector::parse(".playByPlay__logo img").unwrap(),
			score_event: Selector::parse(".playByPlay__details--scoreType").unwrap(),
			time: Selector::parse(".playByPlay__details--timeStamp").unwrap(),
			score_desc: Selector::parse(".playByPlay__details--drives--headline").unwrap(),
		}
	}
}

#[derive(Debug)]
pub struct GameScoringSummary {
	pub quarter: String,
	pub team: String,
	pub score_event: String,
	pub time: String,
	pub score_desc: String,
}

impl fmt::Display for GameScoringSummary {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(
			f,
			"Quarter: {}\nTeam: {}\nScore Event: {}\nTime: {}\nScore Description: {}",
			self.quarter, self.team, self.score_event, self.time, self.score_desc
		)
	}
}

pub struct NflScoringSummaryIterator<'a> {
	quarter_iter: scraper::html::Select<'a, 'a>,
	current_quarter: Option<String>,
	scoring_iter: Option<scraper::element_ref::Select<'a, 'a>>,
	selectors: &'a NflScoringSummarySelectors,
}

impl<'a> NflScoringSummaryIterator<'a> {
	pub fn new(document: &'a Html, selectors: &'a NflScoringSummarySelectors) -> Self {
		NflScoringSummaryIterator {
			quarter_iter: document.select(&selectors.quarter_section),
			current_quarter: None,
			scoring_iter: None,
			selectors,
		}
	}
}

impl<'a> Iterator for NflScoringSummaryIterator<'a> {
	type Item = GameScoringSummary;

	fn next(&mut self) -> Option<Self::Item> {
		loop {
			if let Some(scoring_iter) = &mut self.scoring_iter {
				if let Some(score) = scoring_iter.next() {
					let team = score
						.select(&self.selectors.team_logo)
						.next()
						.and_then(|img| img.value().attr("alt").map(String::from))
						.unwrap();
					let score_event = score.select(&self.selectors.score_event).next().map(|el| el.inner_html()).unwrap_or_default();
					let time = score.select(&self.selectors.time).next().map(|el| el.inner_html()).unwrap_or_default();
					let score_desc = score.select(&self.selectors.score_desc).next().map(|el| el.inner_html()).unwrap_or_default();

					return Some(GameScoringSummary {
						quarter: self.current_quarter.clone().unwrap_or_default(),
						team,
						score_event,
						time,
						score_desc,
					});
				}
			}

			match self.quarter_iter.next() {
				Some(quarter) => {
					self.current_quarter = quarter.select(&self.selectors.quarter).next().map(|el| el.inner_html());

					self.scoring_iter = Some(quarter.select(&self.selectors.scoring_section));
				}
				None => return None,
			}
		}
	}
}

pub fn parse_nfl_scoring_summary<'a>(document: &'a Html, selectors: &'a NflScoringSummarySelectors) -> NflScoringSummaryIterator<'a> {
	NflScoringSummaryIterator::new(document, selectors)
}

// Example usage
// pub fn process_nfl_schedule(html: &str) {
// 	let document = Html::parse_document(html);
// 	let selectors = NflScoringSummarySelectors::new();
//
// 	for game in parse_nfl_game_schedule(&document, &selectors) {
// 		println!("{}", game);
// 	}
