use std::path::{Path, PathBuf};
use tokio::fs;

pub struct FileSystem {
	root: PathBuf,
}

impl FileSystem {
	pub fn new<P: AsRef<Path>>(root: P) -> Result<Self, std::io::Error> {
		let root = root.as_ref().to_path_buf();
		if !root.exists() {
			std::fs::create_dir_all(&root)?;
		}
		Ok(Self { root })
	}

	pub async fn add<P: AsRef<Path>>(&self, path: P) -> Result<(), std::io::Error> {
		let path = self.root.join(path);
		if path.is_file() {
			Ok(())
		} else if path.is_dir() {
			fs::create_dir_all(path).await
		} else {
			fs::File::create(path).await.map(|_| ())
		}
	}

	pub async fn remove<P: AsRef<Path>>(&self, path: P) -> Result<(), std::io::Error> {
		let path = self.root.join(path);
		if path.is_file() {
			fs::remove_file(path).await
		} else if path.is_dir() {
			fs::remove_dir_all(path).await
		} else {
			Ok(())
		}
	}
}
