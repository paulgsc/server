pub mod error;
pub mod query_selectors;
pub mod schema;

use scraper::Html;
use file_reader::{FileReader, FileReaderError};

pub fn read_html_file(file_path: &str) -> Result<Html, FileReaderError> {
	let reader = FileReader::new(file_path, "html")?;
	let html_content = reader.read_content()?;
	Ok(Html::parse_document(&html_content))
}
