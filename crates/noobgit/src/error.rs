use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum FileSystemError {
	#[error("IO error: {0}")]
	Io(#[from] std::io::Error),

	#[error("Path not found: {0}")]
	PathNotFound(PathBuf),

	#[error("Path is not a directory: {0}")]
	NotADirectory(PathBuf),

	#[error("Invalid path: {0}")]
	InvalidPath(PathBuf),

	#[error("Permission denied: {0}")]
	PermissionDenied(PathBuf),

	#[error("File already exists: {0}")]
	FileAlreadyExists(PathBuf),

	#[error("Directory not empty: {0}")]
	DirectoryNotEmpty(PathBuf),

	#[error("Unexpected error: {0}")]
	Unexpected(String),
}

impl FileSystemError {
	pub fn status_code(&self) -> u16 {
		match self {
			Self::Io(_) => 500,
			Self::PathNotFound(_) => 404,
            Self::NotADirectory(_) => 500,
			Self::InvalidPath(_) => 400,
			Self::PermissionDenied(_) => 403,
			Self::FileAlreadyExists(_) => 409,
			Self::DirectoryNotEmpty(_) => 409,
			Self::Unexpected(_) => 500,
		}
	}
}

pub type Result<T> = std::result::Result<T, FileSystemError>;
