use nfl_play_parser::query_selectors::{parse_nfl_scoring_summary, NflScoringSummarySelectors};
use nfl_play_parser::read_html_file;

fn main() {
	let file_path = "examples/scoring.html";

	match read_html_file(file_path) {
		Ok(document) => {
			let selectors = NflScoringSummarySelectors::new();

			for game in parse_nfl_scoring_summary(&document, &selectors) {
				println!("{}", game);
			}
		}
		Err(e) => {
			eprintln!("Failed to read the HTML file: {}", e);
		}
	}
}
