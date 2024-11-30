use scraper::{Html, Selector};
use std::fmt;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ScheduleParserError {
	#[error("Missing NFL Year token")]
	MissingNFLYear,
}

pub struct NflGameScheduleSelectors {
	pub schedule_section: Selector,
	pub nfl_year: Selector,
	pub date_header: Selector,
	pub matchup_strip: Selector,
	pub schedule_desc: Selector,
	pub game_status: Selector,
}

impl NflGameScheduleSelectors {
	pub fn new() -> Self {
		NflGameScheduleSelectors {
			schedule_section: Selector::parse(".nfl-o-matchup-group").unwrap(),
			nfl_year: Selector::parse(".nfl-o-page-title").unwrap(),
			date_header: Selector::parse(".d3-o-section-title").unwrap(),
			matchup_strip: Selector::parse(".nfl-c-matchup-strip").unwrap(),
			schedule_desc: Selector::parse("a.nfl-c-matchup-strip__left-area").unwrap(),
			game_status: Selector::parse(".nfl-c-matchup-strip__game-info p").unwrap(),
		}
	}
}

#[derive(Debug)]
pub struct GameInfo {
	pub nfl_year: String,
	pub date: String,
	pub schedule_desc: String,
	pub status: String,
}

impl fmt::Display for GameInfo {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(
			f,
			"NFL Year: {} Date: {}\nStatus: {}\nSchedule_desc: {}",
			self.nfl_year, self.date, self.status, self.schedule_desc,
		)
	}
}

pub struct NflGameScheduleIterator<'a> {
	schedule_iter: scraper::html::Select<'a, 'a>,
	nfl_year: String,
	current_date: Option<String>,
	matchup_iter: Option<scraper::element_ref::Select<'a, 'a>>,
	selectors: &'a NflGameScheduleSelectors,
}

impl<'a> NflGameScheduleIterator<'a> {
	pub fn new(document: &'a Html, selectors: &'a NflGameScheduleSelectors) -> Result<Self, ScheduleParserError> {
		let nfl_year = document
			.select(&selectors.nfl_year)
			.next()
			.ok_or_else(|| ScheduleParserError::MissingNFLYear)
			.map(|el| el.inner_html())?;

		Ok(NflGameScheduleIterator {
			schedule_iter: document.select(&selectors.schedule_section),
			nfl_year,
			current_date: None,
			matchup_iter: None,
			selectors,
		})
	}
}

impl<'a> Iterator for NflGameScheduleIterator<'a> {
	type Item = GameInfo;

	fn next(&mut self) -> Option<Self::Item> {
		loop {
			if let Some(matchup_iter) = &mut self.matchup_iter {
				if let Some(matchup) = matchup_iter.next() {
					let schedule_desc = matchup
						.select(&self.selectors.schedule_desc)
						.next()
						.and_then(|a| a.value().attr("href").map(String::from))
						.unwrap_or_default();

					let status = matchup.select(&self.selectors.game_status).next().map(|el| el.inner_html()).unwrap_or_default();

					return Some(GameInfo {
						nfl_year: self.nfl_year.clone(),
						date: self.current_date.clone().unwrap(),
						schedule_desc,
						status,
					});
				}
			}

			match self.schedule_iter.next() {
				Some(section) => {
					self.current_date = section.select(&self.selectors.date_header).next().map(|el| el.inner_html());

					self.matchup_iter = Some(section.select(&self.selectors.matchup_strip));
				}
				None => return None,
			}
		}
	}
}

pub fn parse_nfl_game_schedule<'a>(document: &'a Html, selectors: &'a NflGameScheduleSelectors) -> Result<NflGameScheduleIterator<'a>, ScheduleParserError> {
	NflGameScheduleIterator::new(document, selectors).map_err(|_| ScheduleParserError::MissingNFLYear)
}

// Example usage
// pub fn process_nfl_schedule(html: &str) {
// 	let document = Html::parse_document(html);
// 	let selectors = NflGameScheduleSelectors::new();
//
// 	for game in parse_nfl_game_schedule(&document, &selectors) {
// 		println!("{}", game);
// 	}
// }
