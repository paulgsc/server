use crate::error::{FileSystemError, Result};
use std::path::{Path, PathBuf};
use tokio::fs;

pub struct FileSystem {
	root: PathBuf,
}

impl FileSystem {
	pub async fn new<P: AsRef<Path>>(root: P) -> Result<Self> {
		let root = root.as_ref().to_path_buf();
		match (root.exists(), root.is_dir()) {
			(false, _) => Err(FileSystemError::PathNotFound(root)),
			(true, false) => Err(FileSystemError::NotADirectory(root)),
			(true, true) => Ok(Self { root }),
		}
	}

	pub async fn add<P: AsRef<Path>>(&self, path: P) -> Result<()> {
		let path = self.root.join(path);

		if let Ok(metadata) = fs::metadata(&path).await {
			if metadata.is_file() {
				return Ok(());
			} else if metadata.is_dir() {
				return fs::create_dir_all(path).await.map_err(FileSystemError::from);
			}
		}

		fs::File::create(path).await.map(|_| ()).map_err(FileSystemError::from)
	}

	pub async fn remove<P: AsRef<Path>>(&self, path: P) -> Result<()> {
		let path = self.root.join(path);

		if let Ok(metadata) = fs::metadata(&path).await {
			if metadata.is_file() {
				return fs::remove_file(path).await.map_err(FileSystemError::from);
			} else if metadata.is_dir() {
				return fs::remove_dir_all(path).await.map_err(FileSystemError::from);
			}
		}

		Err(FileSystemError::PathNotFound(path))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use tempfile::TempDir;
	use tokio::test;

	#[test]
	async fn test_new_filesystem() {
		let temp_dir = TempDir::new().unwrap();
		let fs = FileSystem::new(temp_dir.path()).await.unwrap();
		assert_eq!(fs.root, temp_dir.path());
	}

	#[test]
	async fn test_add_file() {
		let temp_dir = TempDir::new().unwrap();
		let fs = FileSystem::new(temp_dir.path()).await.unwrap();

		fs.add("test.txt").await.unwrap();
		assert!(temp_dir.path().join("test.txt").exists());
	}

	#[test]
	async fn test_add_directory() {
		let temp_dir = TempDir::new().unwrap();
		let fs = FileSystem::new(temp_dir.path()).await.unwrap();

		fs.add("test_dir").await.unwrap();
		assert!(temp_dir.path().join("test_dir").is_dir());
	}

	#[test]
	async fn test_remove_file() {
		let temp_dir = TempDir::new().unwrap();
		let fs = FileSystem::new(temp_dir.path()).await.unwrap();

		fs.add("test.txt").await.unwrap();
		fs.remove("test.txt").await.unwrap();
		assert!(!temp_dir.path().join("test.txt").exists());
	}

	#[test]
	async fn test_remove_directory() {
		let temp_dir = TempDir::new().unwrap();
		let fs = FileSystem::new(temp_dir.path()).await.unwrap();

		fs.add("test_dir").await.unwrap();
		fs.remove("test_dir").await.unwrap();
		assert!(!temp_dir.path().join("test_dir").exists());
	}

	#[test]
	async fn test_add_existing_file() {
		let temp_dir = TempDir::new().unwrap();
		let fs = FileSystem::new(temp_dir.path()).await.unwrap();

		fs.add("test.txt").await.unwrap();
		fs.add("test.txt").await.unwrap(); // Should not error
		assert!(temp_dir.path().join("test.txt").exists());
	}

	#[test]
	async fn test_remove_nonexistent_path() {
		let temp_dir = TempDir::new().unwrap();
		let fs = FileSystem::new(temp_dir.path()).await.unwrap();

		let result = fs.remove("nonexistent.txt").await;
		assert!(result.is_err(), "Expected an error, but got Ok");
		assert!(matches!(result, Err(FileSystemError::PathNotFound(_))), "Expected PathNotFound error, but got {:?}", result);
	}
}
