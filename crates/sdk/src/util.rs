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
