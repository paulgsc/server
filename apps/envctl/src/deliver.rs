//! Delivery (#614): each app's decrypted file, for its compose service's
//! `env_file` only, on tmpfs and nowhere else, and gone once compose has
//! read it. A running container keeps the environment it started with.

use crate::sops::Plain;
use anyhow::{bail, Context, Result};
use indexmap::IndexMap;
use std::{
	fs::{self, OpenOptions},
	io::Write as _,
	os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _},
	path::{Path, PathBuf},
};

/// Where the decrypted files go: `$XDG_RUNTIME_DIR/server`, or `given`.
/// Either must be on tmpfs (or ramfs): a decrypted value never reaches a
/// disk.
pub(crate) fn env_dir(given: Option<&Path>, shell: &IndexMap<String, String>) -> Result<PathBuf> {
	let dir = match given {
		Some(dir) => dir.to_path_buf(),
		None => match shell.get("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty()) {
			Some(runtime) => Path::new(runtime).join("server"),
			None => bail!("XDG_RUNTIME_DIR is not set, so there is no tmpfs to decrypt into; log in through a session that sets it, or pass --env-dir on a tmpfs"),
		},
	};
	let mounts = fs::read_to_string("/proc/self/mounts").context("reading /proc/self/mounts to see what holds the decrypted files")?;
	let parent = dir.ancestors().find(|ancestor| ancestor.exists()).unwrap_or_else(|| Path::new("/"));
	let parent = fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
	match filesystem(&mounts, &parent) {
		Some("tmpfs" | "ramfs") => Ok(dir),
		Some(other) => bail!("{} is on {other}, not tmpfs: a decrypted value is never written to a disk", dir.display()),
		None => bail!("no mount holds {}", dir.display()),
	}
}

/// The type of the filesystem `path` (canonical) is on, from
/// `/proc/self/mounts`: the longest mount point above it.
pub(crate) fn filesystem<'a>(mounts: &'a str, path: &Path) -> Option<&'a str> {
	mounts
		.lines()
		.filter_map(|line| {
			let mut fields = line.split_whitespace();
			let (_, point, kind) = (fields.next()?, fields.next()?, fields.next()?);
			// /proc/self/mounts writes a space in a mount point as \040.
			let point = point.replace("\\040", " ");
			path.starts_with(&point).then(|| (Path::new(&point).components().count(), kind))
		})
		.max_by_key(|(depth, _)| *depth)
		.map(|(_, kind)| kind)
}

/// One line compose's `env_file` reads back exactly: single-quoted, which
/// compose takes literally but for `\'` (a quote). A value it cannot give
/// back as it is is refused by name, never shown.
pub(crate) fn compose_line(name: &str, value: &str) -> Result<String> {
	if value.ends_with('\\') || value.contains("\\'") {
		bail!("{name}'s value ends in a backslash or holds one before a quote, which compose's env_file cannot read back as it is");
	}
	Ok(format!("{name}='{}'\n", value.replace('\'', "\\'")))
}

/// The decrypted files of one run; each is removed when this is dropped.
#[derive(Debug)]
pub(crate) struct Delivered {
	files: Vec<PathBuf>,
}

impl Delivered {
	/// `<dir>/<app>.env` for each app, mode 600 in a mode 700 directory.
	/// A file a run before this one left (one killed mid-way) is replaced.
	pub(crate) fn write(dir: &Path, apps: &IndexMap<String, Plain>) -> Result<Self> {
		fs::DirBuilder::new()
			.recursive(true)
			.mode(0o700)
			.create(dir)
			.with_context(|| format!("creating {}", dir.display()))?;
		let mut delivered = Self { files: Vec::new() };
		for (app, entries) in apps {
			let path = dir.join(format!("{app}.env"));
			let mut text = String::new();
			for (name, value) in entries {
				text.push_str(&compose_line(name, value)?);
			}
			match fs::remove_file(&path) {
				Ok(()) => {}
				Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
				Err(err) => return Err(err).with_context(|| format!("removing a stale {}", path.display())),
			}
			let mut file = OpenOptions::new()
				.write(true)
				.create_new(true)
				.mode(0o600)
				.open(&path)
				.with_context(|| format!("creating {}", path.display()))?;
			delivered.files.push(path.clone());
			file.write_all(text.as_bytes()).with_context(|| format!("writing {}", path.display()))?;
		}
		Ok(delivered)
	}
}

impl Drop for Delivered {
	fn drop(&mut self) {
		for file in &self.files {
			let _ = fs::remove_file(file);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{compose_line, filesystem, Delivered};
	use crate::sops::Plain;
	use indexmap::IndexMap;
	use std::{os::unix::fs::PermissionsExt as _, path::Path};

	#[test]
	fn the_deepest_mount_decides() {
		let mounts = "/dev/vda / ext4 rw 0 0\ntmpfs /run tmpfs rw 0 0\ntmpfs /run/user/1000 tmpfs rw 0 0\n/dev/vdb /run/disk ext4 rw 0 0\n";
		assert_eq!(filesystem(mounts, Path::new("/run/user/1000/server")), Some("tmpfs"));
		assert_eq!(filesystem(mounts, Path::new("/run/disk/x")), Some("ext4"));
		assert_eq!(filesystem(mounts, Path::new("/home/me/server")), Some("ext4"));
		assert_eq!(filesystem(mounts, Path::new("/runner")), Some("ext4"), "a prefix of a name is not a parent");
	}

	#[test]
	fn a_line_compose_reads_back_exactly() {
		assert_eq!(compose_line("A", "it's $HOME # not a comment").unwrap(), "A='it\\'s $HOME # not a comment'\n");
		assert_eq!(compose_line("PEM", "a\nb").unwrap(), "PEM='a\nb'\n");
		assert!(compose_line("A", "ends\\").is_err());
		let refused = compose_line("A", "x\\'y").unwrap_err().to_string();
		assert!(refused.starts_with("A's value") && !refused.contains("x\\'y"), "named, never shown");
	}

	#[test]
	fn the_files_are_private_and_gone_after() {
		let dir = std::env::temp_dir().join(format!("envctl-deliver-{}", std::process::id()));
		let apps = IndexMap::from([("probe".to_owned(), Plain::from([("A".to_owned(), "1".to_owned())]))]);
		std::fs::create_dir_all(&dir).unwrap();
		std::fs::write(dir.join("probe.env"), "STALE=1\n").unwrap();
		{
			let delivered = Delivered::write(&dir, &apps).unwrap();
			let path = dir.join("probe.env");
			assert_eq!(std::fs::read_to_string(&path).unwrap(), "A='1'\n", "a stale file is replaced");
			assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
			drop(delivered);
			assert!(!path.exists());
		}
		std::fs::remove_dir_all(&dir).unwrap();
	}
}
