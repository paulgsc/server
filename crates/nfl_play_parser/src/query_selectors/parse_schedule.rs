use scraper::{Html, Selector};
use std::fmt;

pub struct NflGameScheduleSelectors {
	pub schedule_section: Selector,
	pub date_header: Selector,
	pub matchup_strip: Selector,
	pub schedule_desc: Selector,
	pub game_status: Selector,
}

impl NflGameScheduleSelectors {
	pub fn new() -> Self {
		NflGameScheduleSelectors {
			schedule_section: Selector::parse(".nfl-o-matchup-group").unwrap(),
			date_header: Selector::parse(".d3-o-section-title").unwrap(),
			matchup_strip: Selector::parse(".nfl-c-matchup-strip").unwrap(),
			schedule_desc: Selector::parse("a.nfl-c-matchup-strip__left-area").unwrap(),
			game_status: Selector::parse(".nfl-c-matchup-strip__game-info p").unwrap(),
		}
	}
}

#[derive(Debug)]
pub struct GameInfo {
	pub date: String,
	pub schedule_desc: String,
	pub status: String,
}

impl fmt::Display for GameInfo {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "Date: {}\nStatus: {}\nSchedule_desc: {}", self.date, self.status, self.schedule_desc,)
	}
}

pub struct NflGameScheduleIterator<'a> {
	schedule_iter: scraper::html::Select<'a, 'a>,
	current_date: Option<String>,
	matchup_iter: Option<scraper::element_ref::Select<'a, 'a>>,
	selectors: &'a NflGameScheduleSelectors,
}

impl<'a> NflGameScheduleIterator<'a> {
	pub fn new(document: &'a Html, selectors: &'a NflGameScheduleSelectors) -> Self {
		NflGameScheduleIterator {
			schedule_iter: document.select(&selectors.schedule_section),
			current_date: None,
			matchup_iter: None,
			selectors,
		}
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
						date: self.current_date.clone().unwrap_or_default(),
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

pub fn parse_nfl_game_schedule<'a>(document: &'a Html, selectors: &'a NflGameScheduleSelectors) -> NflGameScheduleIterator<'a> {
	NflGameScheduleIterator::new(document, selectors)
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
