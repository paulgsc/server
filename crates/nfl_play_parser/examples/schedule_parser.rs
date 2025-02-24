use nfl_play_parser::query_selectors::{parse_nfl_game_schedule, NflGameScheduleSelectors};
use nfl_play_parser::read_html_file;

fn main() {
	let file_path = "examples/schedule.html";

	match read_html_file(file_path) {
		Ok(document) => {
			let selectors = NflGameScheduleSelectors::new();

			match parse_nfl_game_schedule(&document, &selectors) {
				Ok(iterator) => {
					for schedule in iterator {
						println!("{}", schedule);
					}
				}
				Err(e) => {
					eprintln!("{}", e);
				}
			}
		}
		Err(e) => {
			eprintln!("Failed to read the HTML file: {}", e);
		}
	}
}
