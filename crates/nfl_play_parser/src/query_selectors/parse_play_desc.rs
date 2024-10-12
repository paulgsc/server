use scraper::{Html, Selector, ElementRef};
use crate::query_selectors::PlaySelectors;

pub struct ParsedSelectors {
    pub play_list: Selector,
    pub headline: Selector,
    pub description: Selector,
}

impl ParsedSelectors {
    pub fn new() -> Self {
        ParsedSelectors {
            play_list: Selector::parse(PlaySelectors::PlayList.selector()).unwrap(),
            headline: Selector::parse(PlaySelectors::Headline.selector()).unwrap(),
            description: Selector::parse(PlaySelectors::Description.selector()).unwrap(),
        }
    }
}

pub struct PlayDescriptionIterator<'a> {
    play_iter: scraper::html::Select<'a, 'a>,
    selectors: &'a ParsedSelectors,
}

impl<'a> PlayDescriptionIterator<'a> {
    fn new(document: &'a Html, selectors: &'a ParsedSelectors) -> Self {
        PlayDescriptionIterator {
            play_iter: document.select(&selectors.play_list),
            selectors,
        }
    }
}

impl<'a> Iterator for PlayDescriptionIterator<'a> {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        self.play_iter.next().map(|play| {
            let headline = play.select(&self.selectors.headline).next().map(|h| h.inner_html()).unwrap_or_default();
            let description = play.select(&self.selectors.description).next().map(|d| d.inner_html()).unwrap_or_default();
            format!("{} ||| {}", headline, description)
        })
    }
}

pub fn parse_play_descriptions<'a>(document: &'a Html, selectors: &'a ParsedSelectors) -> PlayDescriptionIterator<'a> {
    PlayDescriptionIterator::new(document, selectors)
}

// Example usage (not part of the function, just for demonstration):
// fn main() {
//     let html_content = std::fs::read_to_string("path/to/your/file.html").expect("Failed to read file");
//     let document = Html::parse_document(&html_content);
//     let parsed_selectors = ParsedSelectors::new();
//     
//     for description in parse_play_descriptions(&document, &parsed_selectors) {
//         println!("{}", description);
//     }
// }
