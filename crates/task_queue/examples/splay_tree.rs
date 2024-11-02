use std::io::{self, Write as IoWrite};
use task_queue::trees::splay_tree::SplayTree;

fn main() -> io::Result<()> {
	println!("SplayTree Demo\n");

	// Create a new tree
	let mut tree = SplayTree::new();
	println!("Created empty tree:");
	println!("{}", tree);

	// Helper function to pause and wait for user input
	fn pause() -> io::Result<()> {
		print!("\nPress Enter to continue...");
		io::stdout().flush()?;
		let mut buffer = String::new();
		io::stdin().read_line(&mut buffer)?;
		Ok(())
	}

	// Demonstrate insertions
	println!("\nInserting values...");
	let operations = [(5, "five"), (3, "three"), (7, "seven"), (1, "one"), (6, "six"), (9, "nine")];

	for (key, value) in operations {
		println!("\nInserting ({}, {})", key, value);
		tree.insert(key, value);
		println!("\nTree after insertion:");
		println!("{}", tree);
		pause()?;
	}

	// Demonstrate lookups
	println!("\nLooking up some values...");
	for key in [3, 6, 8] {
		println!("\nLooking up key: {}", key);
		if let Some(value) = tree.get(&key) {
			println!("Found: {}", value);
		} else {
			println!("Not found");
		}
		println!("\nTree after lookup:");
		println!("{}", tree);
		pause()?;
	}

	// Demonstrate removals
	println!("\nRemoving some values...");
	for key in [1, 7, 5] {
		println!("\nRemoving key: {}", key);
		if let Some(value) = tree.remove(&key) {
			println!("Removed value: {}", value);
		} else {
			println!("Key not found");
		}
		println!("\nTree after removal:");
		println!("{}", tree);
		pause()?;
	}

	// Demonstrate min/max operations
	println!("\nDemonstrating min/max operations...");

	if let Some((key, value)) = tree.get_min() {
		println!("\nMinimum: ({}, {})", key, value);
		println!("\nTree after get_min:");
		println!("{}", tree);
	}
	pause()?;

	if let Some((key, value)) = tree.get_max() {
		println!("\nMaximum: ({}, {})", key, value);
		println!("\nTree after get_max:");
		println!("{}", tree);
	}
	pause()?;

	// Demonstrate iteration
	println!("\nIterating through the tree:");
	println!("Elements in order:");
	for (key, value) in tree.into_iter() {
		println!("({}, {})", key, value);
	}

	Ok(())
}
