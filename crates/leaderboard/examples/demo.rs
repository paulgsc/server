use leaderboard::*;

fn main() {
	example();
}

// Example usage
pub fn example() {
	let data = vec![
		("Lions", 1.0),
		("Packers", 1.0),
		("Falcons", 0.875),
		("Buccaneers", 0.75),
		("Chargers", 0.75),
		("49ers", 0.75),
		("Texans", 0.75),
		("Eagles", 0.75),
		("Steelers", 0.75),
		("Bears", 0.625),
		("Ravens", 0.625),
		("Bengals", 0.625),
		("Jets", 0.5),
		("Colts", 0.5),
		("Bills", 0.5),
		("Titans", 0.5),
		("Giants", 0.375),
		("Commanders", 0.375),
		("Jaguars", 0.375),
		("Browns", 0.25),
		("Seahawks", 0.25),
		("Patriots", 0.25),
		("Raiders", 0.25),
		("Broncos", 0.25),
		("Saints", 0.25),
		("Panthers", 0.125),
		("Cardinals", 0.0),
		("Cowboys", 0.0),
	];

	// Create a pyramid-style leader board with max on top
	let mut leader_board = LeaderBoard::new(ChartStyle::Pyramid, SortDirection::MaxOnTop);
	let elements = leader_board.build_leaderboard_data(data);
	leader_board.build(elements);

	println!("Pyramid Style (Max on Top):");
	println!("{}", leader_board);

	// Create a brick wall style leader board with max on top
	leader_board.set_style(ChartStyle::BrickWall);
	println!("Brick Wall Style (Max on Top):");
	println!("{}", leader_board);

	// Change to min on top
	leader_board.set_sort_direction(SortDirection::MinOnTop);
	println!("Brick Wall Style (Min on Top):");
	println!("{}", leader_board);
}
