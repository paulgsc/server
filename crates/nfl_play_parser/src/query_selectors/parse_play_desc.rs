use crate::query_selectors::PlaySelectors;
use scraper::{ElementRef, Html, Selector};
use std::fmt;

pub struct ParsedSelectors {
	pub playlist: Selector,
	pub play_list_item: Selector,
	pub headline: Selector,
	pub description: Selector,
	pub team_logo: Selector,
}

impl ParsedSelectors {
	pub fn new() -> Self {
		ParsedSelectors {
			playlist: Selector::parse(PlaySelectors::PlayList.selector()).unwrap(),
			play_list_item: Selector::parse(PlaySelectors::PlayListItem.selector()).unwrap(),
			headline: Selector::parse(PlaySelectors::Headline.selector()).unwrap(),
			description: Selector::parse(PlaySelectors::Description.selector()).unwrap(),
			team_logo: Selector::parse(PlaySelectors::TeamLogo.selector()).unwrap(),
		}
	}
}

pub struct PlayDescription {
	pub team_name: Option<String>,
	pub headline: Option<String>,
	pub description: Option<String>,
}

impl fmt::Display for PlayDescription {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		// Provide default values if any of the fields are None
		let team_name = self.team_name.as_deref().unwrap_or("Unknown Team");
		let headline = self.headline.as_deref().unwrap_or("No Headline");
		let description = self.description.as_deref().unwrap_or("No Description");

		// Write out the formatted string
		write!(f, "Team: {}\nHeadline: {}\nDescription: {}", team_name, headline, description)
	}
}

pub struct PlayDescriptionIterator<'a> {
	playlist_iter: scraper::html::Select<'a, 'a>,
	current_playlist: Option<ElementRef<'a>>,
	current_team_name: Option<String>,
	play_iter: Option<scraper::element_ref::Select<'a, 'a>>,
	selectors: &'a ParsedSelectors,
}

impl<'a> PlayDescriptionIterator<'a> {
	pub fn new(document: &'a Html, selectors: &'a ParsedSelectors) -> Self {
		PlayDescriptionIterator {
			playlist_iter: document.select(&selectors.playlist),
			current_playlist: None,
			current_team_name: None,
			play_iter: None,
			selectors,
		}
	}
}

impl<'a> Iterator for PlayDescriptionIterator<'a> {
	type Item = PlayDescription;

	fn next(&mut self) -> Option<Self::Item> {
		loop {
			if let Some(play_iter) = &mut self.play_iter {
				if let Some(play) = play_iter.next() {
					let headline = play.select(&self.selectors.headline).next().map(|h| h.inner_html());
					let description = play.select(&self.selectors.description).next().map(|d| d.inner_html());
					return Some(PlayDescription {
						team_name: self.current_team_name.clone(),
						headline,
						description,
					});
				}
			}

			self.current_playlist = self.playlist_iter.next();
			if let Some(playlist) = self.current_playlist {
				self.current_team_name = playlist.select(&self.selectors.team_logo).next().and_then(|img| img.value().attr("alt").map(String::from));

				self.play_iter = Some(playlist.select(&self.selectors.play_list_item));
			} else {
				return None;
			}
		}
	}
}

pub fn parse_play_descriptions<'a>(document: &'a Html, selectors: &'a ParsedSelectors) -> PlayDescriptionIterator<'a> {
	PlayDescriptionIterator::new(document, selectors)
}

// Example usage:
// fn main() {
//     let html_content = std::fs::read_to_string("path/to/your/file.html").expect("Failed to read file");
//     let document = Html::parse_document(&html_content);
//     let parsed_selectors = ParsedSelectors::new();
//
//     for description in parse_play_descriptions(&document, &parsed_selectors) {
//         println!("{}", description);
//     }
// }
