pub mod config;
pub mod core;

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::collections::HashSet;
use std::fs::File;
use std::io::BufReader;

#[derive(Debug)]
pub struct Element {
	pub name: String,
	pub attributes: Vec<(String, String)>,
	pub content: String,
	pub children: Vec<Element>,
}

pub fn extract_elements(file_path: &str, selectors: &[&str]) -> Result<Vec<Element>, Box<dyn std::error::Error>> {
	let file = File::open(file_path)?;
	let buf_reader = BufReader::new(file);
	let mut reader = Reader::from_reader(buf_reader);
	// No trim_text call here
	let mut buf = Vec::new();
	let mut stack = Vec::new();
	let mut extracted_elements = Vec::new();
	let selector_set: HashSet<_> = selectors.iter().map(|&s| s.to_string()).collect();

	let mut current_depth = 0;
	let mut capture_depths = Vec::new();

	loop {
		match reader.read_event_into(&mut buf)? {
			Event::Start(e) => {
				current_depth += 1;
				let name = std::str::from_utf8(e.name().as_ref())?.to_string();
				let attributes = e
					.attributes()
					.filter_map(|a| {
						a.ok()
							.map(|attr| (std::str::from_utf8(attr.key.as_ref()).unwrap().to_string(), attr.unescape_value().unwrap().to_string()))
					})
					.collect();

				let element = Element {
					name: name.clone(),
					attributes,
					content: String::new(),
					children: Vec::new(),
				};

				if selector_set.contains(&name) || !capture_depths.is_empty() {
					if capture_depths.is_empty() {
						capture_depths.push(current_depth);
					}
					stack.push(element);
				}
			}
			Event::Text(e) => {
				if !capture_depths.is_empty() {
					if let Some(element) = stack.last_mut() {
						// Manually trim whitespace text
						let text = e.unescape()?.trim().to_string();
						if !text.is_empty() {
							element.content.push_str(&text);
						}
					}
				}
			}
			Event::End(_) => {
				if !capture_depths.is_empty() && current_depth == *capture_depths.last().unwrap() {
					if let Some(element) = stack.pop() {
						if stack.is_empty() {
							extracted_elements.push(element);
							capture_depths.pop();
						} else if let Some(parent) = stack.last_mut() {
							parent.children.push(element);
						}
					}
				}
				current_depth -= 1;
			}
			Event::Eof => break,
			_ => (),
		}
		buf.clear();
	}

	Ok(extracted_elements)
}
