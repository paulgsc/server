use std::path::{Path as StdPath, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone)]
pub enum GoogleServiceFilePath {
	SecretFilePath(PathBuf), // Store owned PathBuf
}

impl AsRef<StdPath> for GoogleServiceFilePath {
	fn as_ref(&self) -> &StdPath {
		match self {
			GoogleServiceFilePath::SecretFilePath(path) => path.as_ref(),
		}
	}
}

#[derive(Debug, Error)]
pub enum SecretFilePathError {
	#[error("Invalid file extension: expected .json, got {extension}")]
	InvalidExtension { extension: String },

	#[error("Invalid filename: expected client_secret_file.json, got {filename}")]
	InvalidFilename { filename: String },

	#[error("Missing credentials file: {path}")]
	MissingFile { path: String },

	#[error("Not a file (e.g., is a directory): {path}")]
	NotAFile { path: String },
}

impl GoogleServiceFilePath {
	pub fn new(path: String) -> Result<Self, SecretFilePathError> {
		let std_path = StdPath::new(&path);

		// Validate it's not empty
		if path.trim().is_empty() {
			return Err(SecretFilePathError::MissingFile { path: "<empty>".to_string() });
		}

		// Extract filename
		let filename = std_path
			.file_name()
			.and_then(|s| s.to_str())
			.ok_or_else(|| SecretFilePathError::InvalidFilename { filename: path.clone() })?;

		if filename != "client_secret_file.json" {
			return Err(SecretFilePathError::InvalidFilename { filename: filename.to_string() });
		}

		// Validate extension
		if std_path.extension().and_then(|s| s.to_str()) != Some("json") {
			return Err(SecretFilePathError::InvalidExtension { extension: filename.to_string() });
		}

		// Check existence
		if !std_path.exists() {
			return Err(SecretFilePathError::MissingFile { path });
		}

		// Check that it's a file, not a dir
		if !std_path.is_file() {
			return Err(SecretFilePathError::NotAFile { path });
		}

		// All good — store the original path as PathBuf
		Ok(GoogleServiceFilePath::SecretFilePath(std_path.to_path_buf()))
	}

	pub fn as_str(&self) -> &str {
		match self {
			GoogleServiceFilePath::SecretFilePath(path) => path.to_str().expect("Path should be valid Unicode"),
		}
	}
}

pub(crate) fn column_number_to_letter(mut column: u32) -> String {
	let mut result = String::new();
	while column > 0 {
		column -= 1;
		let remainder = column % 26;
		let letter = (remainder as u8 + b'A') as char;
		result.insert(0, letter);
		column /= 26;
	}
	result
}

#[cfg(test)]
mod tests {
	use super::*;

	// Helper function to create test paths
	fn create_test_path(path_str: &str) -> Result<Path, PathPartError> {
		Path::parse(path_str)
	}

	#[test]
	fn test_valid_secret_file_path() {
		let path = create_test_path("/path/to/client_secret_file.json").unwrap();
		let result = GoogleServiceFilePath::new(path);
		assert!(result.is_ok());
	}

	#[test]
	fn test_valid_secret_file_path_no_leading_slash() {
		let path = create_test_path("path/to/client_secret_file.json").unwrap();
		let result = GoogleServiceFilePath::new(path);
		assert!(result.is_ok());
	}

	#[test]
	fn test_invalid_file_extension() {
		let path = create_test_path("/path/to/client_secret_file.txt").unwrap();
		let result = GoogleServiceFilePath::new(path);

		match result {
			Err(SecretFilePathError::InvalidExtension { extension }) => {
				assert_eq!(extension, "txt");
			}
			_ => panic!("Expected InvalidExtension error"),
		}
	}

	#[test]
	fn test_no_file_extension() {
		let path = create_test_path("/path/to/client_secret_file").unwrap();
		let result = GoogleServiceFilePath::new(path);

		match result {
			Err(SecretFilePathError::InvalidExtension { extension }) => {
				assert_eq!(extension, "no extension");
			}
			_ => panic!("Expected InvalidExtension error"),
		}
	}

	#[test]
	fn test_invalid_filename() {
		let path = create_test_path("/path/to/wrong_filename.json").unwrap();
		let result = GoogleServiceFilePath::new(path);

		match result {
			Err(SecretFilePathError::InvalidFilename { filename }) => {
				assert_eq!(filename, "wrong_filename.json");
			}
			_ => panic!("Expected InvalidFilename error"),
		}
	}

	#[test]
	fn test_path_with_dots() {
		let path = create_test_path("/path/./to/../client_secret_file.json").unwrap();
		let result = GoogleServiceFilePath::new(path);
		assert!(result.is_ok());
	}

	#[test]
	fn test_path_with_special_characters() {
		let path = create_test_path("/path with spaces/to/client_secret_file.json").unwrap();
		let result = GoogleServiceFilePath::new(path);
		assert!(result.is_ok());
	}

	#[test]
	fn test_as_str_representation() {
		let path = create_test_path("/path/to/client_secret_file.json").unwrap();
		let secret_path = GoogleServiceFilePath::new(path).unwrap();
		assert_eq!(secret_path.as_str(), "path/to/client_secret_file.json");
	}

	#[test]
	fn test_empty_path() {
		let path = create_test_path("").unwrap();
		let result = GoogleServiceFilePath::new(path);

		match result {
			Err(SecretFilePathError::InvalidFilename { filename }) => {
				assert_eq!(filename, "no filename");
			}
			_ => panic!("Expected InvalidFilename error"),
		}
	}

	#[test]
	fn test_path_with_multiple_extensions() {
		let path = create_test_path("/path/to/client_secret_file.tar.json").unwrap();
		let result = GoogleServiceFilePath::new(path);

		match result {
			Err(SecretFilePathError::InvalidFilename { filename }) => {
				assert_eq!(filename, "client_secret_file.tar.json");
			}
			_ => panic!("Expected InvalidFilename error"),
		}
	}

	#[test]
	fn test_path_case_sensitivity() {
		// Test uppercase extension
		let path = create_test_path("/path/to/client_secret_file.JSON").unwrap();
		let result = GoogleServiceFilePath::new(path);
		assert!(result.is_err());

		// Test uppercase filename
		let path = create_test_path("/path/to/CLIENT_SECRET_FILE.json").unwrap();
		let result = GoogleServiceFilePath::new(path);
		assert!(result.is_err());
	}

	#[test]
	fn test_root_path() {
		let path = create_test_path("/client_secret_file.json").unwrap();
		let result = GoogleServiceFilePath::new(path);
		assert!(result.is_ok());
	}

	#[test]
	fn test_relative_dot_path() {
		let path = create_test_path("./client_secret_file.json").unwrap();
		let result = GoogleServiceFilePath::new(path);
		assert!(result.is_ok());
	}

	// Integration test with GoogleGoogleClient
	#[test]
	fn test_google_sheets_client_creation() {
		let path = create_test_path("/path/to/client_secret_file.json").unwrap();
		let result = GoogleGoogleClient::new("test@example.com".to_string(), path);
		assert!(result.is_ok());

		let path = create_test_path("/path/to/invalid.json").unwrap();
		let result = GoogleGoogleClient::new("test@example.com".to_string(), path);
		assert!(result.is_err());
	}
}
