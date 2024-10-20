pub mod config;
pub mod core;

use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

pub struct HtmlFileChunkIterator {
	reader: BufReader<File>,
	chunk_size: usize,
	total_size: u64,
	current_position: u64,
	buffer: Vec<u8>,
}

impl HtmlFileChunkIterator {
	pub fn new<P: AsRef<Path>>(path: P, chunk_size: usize) -> io::Result<Self> {
		let file = File::open(path)?;
		let total_size = file.metadata()?.len();
		let reader = BufReader::new(file);

		Ok(HtmlFileChunkIterator {
			reader,
			chunk_size,
			total_size,
			current_position: 0,
			buffer: Vec::with_capacity(chunk_size * 2),
		})
	}

	pub fn total_size(&self) -> u64 {
		self.total_size
	}

	fn find_tag_boundary(&self, content: &[u8]) -> Option<usize> {
		let mut depth = 0;
		for (i, &b) in content.iter().enumerate() {
			match b {
				b'<' => {
					depth += 1;
				}
				b'>' => {
					depth -= 1;
					if depth == 0 && i >= self.chunk_size {
						return Some(i + 1);
					}
				}
				_ => {}
			}
		}
        if depth >= self.chunk_size {
            Some(depth)
        } else {
            None
        }
	}
}

impl Iterator for HtmlFileChunkIterator {
	type Item = io::Result<Vec<u8>>;

	fn next(&mut self) -> Option<Self::Item> {
		if self.current_position >= self.total_size {
			return None;
		}

		while self.buffer.len() < self.chunk_size {
			let mut temp_buffer = [0; 1024];
			match self.reader.read(&mut temp_buffer) {
				Ok(0) => break, // EOF
				Ok(n) => {
					self.current_position += n as u64;
					self.buffer.extend_from_slice(&temp_buffer[..n]);
				}
				Err(e) => return Some(Err(e)),
			}
		}

		if self.buffer.is_empty() {
			return None;
		}

		if let Some(split_index) = self.find_tag_boundary(&self.buffer) {
			println!("index_boundary: {}", split_index);
			let result = self.buffer.drain(..split_index).collect();
            println!("result: {:?}", result);
			Some(Ok(result))
		} else {
			let result = self.buffer.drain(..).collect();
			Some(Ok(result))
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::io::Write;
	use tempfile::NamedTempFile;

	fn create_temp_html_file(content: &str) -> NamedTempFile {
		let mut file = NamedTempFile::new().unwrap();
		write!(file, "{}", content).unwrap();
		file
	}

	#[test]
	fn test_valid_html_file() {
		let content = "<html><body><p>Hello</p><p>World</p></body></html>";
		let file = create_temp_html_file(content);
		let iterator = HtmlFileChunkIterator::new(file.path(), 10).unwrap();
		let chunks: Vec<Vec<u8>> = iterator.map(|r| r.unwrap()).collect();

        println!("chunks: {:?}", chunks);
		assert_eq!(chunks.len(), 2);
		assert_eq!(chunks[0], b"<html><body><p>Hello</p>");
		assert_eq!(chunks[1], b"<p>World</p></body></html>");
	}

	#[test]
	fn test_empty_file() {
		let file = create_temp_html_file("");
		let iterator = HtmlFileChunkIterator::new(file.path(), 10).unwrap();
		let chunks: Vec<Vec<u8>> = iterator.map(|r| r.unwrap()).collect();

		assert_eq!(chunks.len(), 0);
	}

	#[test]
	fn test_file_smaller_than_chunk_size() {
		let content = "<p>Small</p>";
		let file = create_temp_html_file(content);
		let iterator = HtmlFileChunkIterator::new(file.path(), 100).unwrap();
		let chunks: Vec<Vec<u8>> = iterator.map(|r| r.unwrap()).collect();

		assert_eq!(chunks.len(), 1);
		assert_eq!(chunks[0], content.as_bytes());
	}

	#[test]
	fn test_invalid_file_path() {
		let result = HtmlFileChunkIterator::new("non_existent_file.html", 10);
		assert!(result.is_err());
	}

	#[test]
	fn test_large_html_element() {
		let content = "<p>".to_string() + &"a".repeat(1000) + "</p>";
		let file = create_temp_html_file(&content);
		let iterator = HtmlFileChunkIterator::new(file.path(), 100).unwrap();
		let chunks: Vec<Vec<u8>> = iterator.map(|r| r.unwrap()).collect();

		assert_eq!(chunks.len(), 1);
		assert_eq!(chunks[0], content.as_bytes());
	}

	#[test]
	fn test_multiple_chunks() {
		let content = "<div>".to_string() + &"<p>Chunk</p>".repeat(10) + "</div>";
		let file = create_temp_html_file(&content);
		let iterator = HtmlFileChunkIterator::new(file.path(), 30).unwrap();
		let chunks: Vec<Vec<u8>> = iterator.map(|r| r.unwrap()).collect();

		assert!(chunks.len() > 1);
		assert_eq!(chunks.concat(), content.as_bytes());
	}
}
