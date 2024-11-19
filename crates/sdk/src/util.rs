use file_reader::config::PathPartError;
use file_reader::core::Path;
use std::path::Path as StdPath;
use thiserror::Error;

#[derive(Debug, Clone)]
pub enum GoogleServiceFilePath {
	SecretFilePath(Path),
}

impl AsRef<StdPath> for GoogleServiceFilePath {
	fn as_ref(&self) -> &StdPath {
		match self {
			GoogleServiceFilePath::SecretFilePath(path) => StdPath::new(path.as_ref()),
		}
	}
}

#[derive(Debug, Error)]
pub enum SecretFilePathError {
	#[error("Invalid file extension: expected .json, got {extension}")]
	InvalidExtension { extension: String },
	#[error("Invalid filename: expected client_secret_file.json, got {filename}")]
	InvalidFilename { filename: String },
	#[error("Path error: {0}")]
	PathError(#[from] PathPartError),
}

impl GoogleServiceFilePath {
	pub fn new(path: String) -> Result<Self, SecretFilePathError> {
		let parsed_path = Path::parse(&path)?;
		let extension = parsed_path.extension().ok_or_else(|| SecretFilePathError::InvalidExtension {
			extension: String::from("no extension"),
		})?;

		if extension != "json" {
			return Err(SecretFilePathError::InvalidExtension { extension: extension.to_string() });
		}

		let filename = parsed_path.filename().ok_or_else(|| SecretFilePathError::InvalidFilename {
			filename: String::from("no filename"),
		})?;

		if filename != "client_secret_file.json" {
			return Err(SecretFilePathError::InvalidFilename { filename: filename.to_string() });
		}

		Ok(GoogleServiceFilePath::SecretFilePath(parsed_path))
	}

	pub fn as_str(&self) -> &str {
		match self {
			GoogleServiceFilePath::SecretFilePath(path) => path.as_ref(),
		}
	}
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
