pub mod config;
pub mod core;

use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

use config::PathPartError;
use core::Path;

#[derive(Debug)]
pub struct FileReader {
	path: Path,
	system_path: PathBuf,
	expected_type: String,
}

#[derive(Debug)]
pub enum FileReaderError {
	InvalidPath(PathPartError),
	FileNotFound,
	InvalidFileType,
	IOError(io::Error),
	NoFileExtension,
}

impl fmt::Display for FileReaderError {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match self {
			FileReaderError::InvalidPath(path) => write!(f, "Invalid path: {}", path),
			FileReaderError::FileNotFound => write!(f, "File not found"),
			FileReaderError::InvalidFileType => write!(f, "Invalid file type"),
			FileReaderError::NoFileExtension => write!(f, "File has no extension"),
			FileReaderError::IOError(e) => write!(f, "IO error: {}", e),
		}
	}
}

impl std::error::Error for FileReaderError {}

impl FileReader {
	pub fn new(path: &str, expected_type: &str) -> Result<Self, FileReaderError> {
		let validated_path = Path::parse(path).map_err(FileReaderError::InvalidPath)?;
		let system_path = PathBuf::from(path);

		Ok(FileReader {
			path: validated_path,
			system_path,
			expected_type: expected_type.to_string(),
		})
	}

	pub fn validate(&self) -> Result<(), FileReaderError> {
		if !self.system_path.exists() {
			return Err(FileReaderError::FileNotFound);
		}

		match self.path.extension() {
			Some(ext) if ext == self.expected_type => Ok(()),
			Some(_) => Err(FileReaderError::InvalidFileType),
			None => Err(FileReaderError::NoFileExtension),
		}
	}

	pub fn read_content(&self) -> Result<String, FileReaderError> {
		self.validate()?;

		let mut file = fs::File::open(&self.system_path).map_err(FileReaderError::IOError)?;

		let mut content = String::new();
		file.read_to_string(&mut content).map_err(FileReaderError::IOError)?;

		Ok(content)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::io::Write;
	use tempfile::{NamedTempFile, TempDir};

	#[test]
	fn test_file_reader() {
		// Create a temporary directory
		let temp_dir = TempDir::new().unwrap();

		// Create a temporary file with content
		let mut file = NamedTempFile::new_in(temp_dir.path()).unwrap();
		writeln!(file, "Test content").unwrap();
		let file_path = file.path().to_str().unwrap();
		let validated_path = Path::parse(file_path).unwrap();
		let file_type = validated_path.extension().unwrap();

		// Test valid file
		let reader = FileReader::new(file_path, file_type).unwrap();
		assert!(reader.validate().is_ok());
		assert_eq!(reader.read_content().unwrap().trim(), "Test content");

		// Test invalid file type
		let reader = FileReader::new(file_path, "pdf").unwrap();
		assert!(matches!(reader.validate(), Err(FileReaderError::InvalidFileType)));

		// Test non-existent file
		let reader = FileReader::new("/path/to/nonexistent/file.txt", "txt").unwrap();
		assert!(matches!(reader.validate(), Err(FileReaderError::FileNotFound)));

		// Test no extension
		let no_ext_file = NamedTempFile::new_in(temp_dir.path()).unwrap();
		let no_ext_path = no_ext_file.path().to_str().unwrap();
		let reader = FileReader::new(no_ext_path, "txt").unwrap();
		assert!(matches!(reader.validate(), Err(FileReaderError::InvalidFileType)));
	}

	#[test]
	fn test_empty_file() {
		let temp_dir = TempDir::new().unwrap();
		let file = NamedTempFile::new_in(temp_dir.path()).unwrap();
		let file_path = file.path().to_str().unwrap();
		let validated_path = Path::parse(file_path).unwrap();
		let file_type = validated_path.extension().unwrap();

		let reader = FileReader::new(file_path, file_type).unwrap();
		assert!(reader.validate().is_ok());
		assert_eq!(reader.read_content().unwrap(), ""); // Empty content
	}
}
