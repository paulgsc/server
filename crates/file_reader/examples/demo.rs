use std::error::Error;
use std::path::Path;

use file_reader::extract_elements;

fn main() {
    if let Err(e) = run_example() {
        eprintln!("Error running example: {}", e);
    }
}

pub fn run_example() -> Result<(), Box<dyn Error>> {
	let demo_file_path = Path::new("examples/demo.html");

	let selectors = vec![
		"div.AccordionPanel",
		"div.AccordionHeader",
		"div.AccordionHeader__Left__Drives",
		"div.AccordionHeader__Right",
		"ul.PlayList",
		"li.PlayListItem",
	];

	let extracted_elements = extract_elements(demo_file_path.to_str().unwrap(), &selectors)?;

	println!("Extracted {} elements:", extracted_elements.len());
	for (i, element) in extracted_elements.iter().enumerate() {
		println!("Element {}:", i + 1);
		println!("  Name: {}", element.name);
		println!("  Attributes:");
		for (key, value) in &element.attributes {
			println!("    {}: {}", key, value);
		}
		println!("  Content: {}", element.content.trim());
		println!("  Number of children: {}", element.children.len());
		println!();
	}

	Ok(())
}
