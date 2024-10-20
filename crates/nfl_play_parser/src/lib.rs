pub mod error;
pub mod query_selectors;
pub mod schema;

use scraper::Html;
use async_trait::async_trait;
use file_reader::HtmlProcessor;
use crate::query_selectors::{parse_play_descriptions, ParsedSelectors};

pub struct PlayHtmlProcessor;

#[async_trait]
impl HtmlProcessor for PlayHtmlProcessor {
    async fn process_html_content(&self, html_content: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let document = Html::parse_document(html_content);
        let parsed_selectors = ParsedSelectors::new();
        for description in parse_play_descriptions(&document, &parsed_selectors) {
            println!("{}", description);
        }
        Ok(())
    }
}


