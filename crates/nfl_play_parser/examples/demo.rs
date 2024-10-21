use nfl_play_parser::query_selectors::{parse_play_descriptions, ParsedSelectors};
use nfl_play_parser::schema::Play;
use nfl_play_parser::read_html_file;

fn main() {
	let file_path = "examples/demo.html";

	match read_html_file(file_path) {
		Ok(document) => {
			let selectors = ParsedSelectors::new();

			for description in parse_play_descriptions(&document, &selectors) {
                match Play::try_from(description) {
                    Ok(play) => println!("{:?}", play),
                    Err(err) => eprintln!("Error parsing play: {}", err),
                }
			}
		}
		Err(e) => {
			eprintln!("Failed to read the HTML file: {}", e);
		}
	}
}
