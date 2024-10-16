use std::path::{Path, PathBuf};
use std::fs;

pub struct FileSystem {
    root: PathBuf,
}

impl FileSystem {
    pub fn new<P: AsRef<Path>>(root: P) -> Result<Self, std::io::Error> {
        let root = root.as_ref().to_path_buf();
        if !root.exists() {
            fs::create_dir_all(&root)?;
        }
        Ok(Self { root })
    }

    pub fn add<P: AsRef<Path>>(&self, path: P) -> Result<(), std::io::Error> {
        let path = self.root.join(path);
        if path.is_file() {
            // File already exists, no need to create
            Ok(())
        } else if path.is_dir() {
            fs::create_dir_all(path)
        } else {
            fs::File::create(path).map(|_| ())
        }
    }

    pub fn remove<P: AsRef<Path>>(&self, path: P) -> Result<(), std::io::Error> {
        let path = self.root.join(path);
        if path.is_file() {
            fs::remove_file(path)
        } else if path.is_dir() {
            fs::remove_dir_all(path)
        } else {
            Ok(()) // Path doesn't exist, no need to remove
        }
    }
}

