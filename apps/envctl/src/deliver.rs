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

/// Where the decrypted files go: `$XDG_RUNTIME_DIR/server`, or `given`,
/// created (mode 700) and returned as its canonical path. It must be
/// absolute and, once created and every link in it resolved, on tmpfs (or
/// ramfs): a decrypted value never reaches a disk.
pub(crate) fn env_dir(given: Option<&Path>, shell: &IndexMap<String, String>) -> Result<PathBuf> {
	let dir = match given {
		Some(dir) => dir.to_path_buf(),
		None => match shell.get("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty()) {
			Some(runtime) => Path::new(runtime).join("server"),
			None => bail!("XDG_RUNTIME_DIR is not set, so there is no tmpfs to decrypt into; log in through a session that sets it, or pass --env-dir on a tmpfs"),
		},
	};
	if !dir.is_absolute() {
		bail!("{} is not an absolute path; compose would read it from infra/compose/", dir.display());
	}
	fs::DirBuilder::new()
		.recursive(true)
		.mode(0o700)
		.create(&dir)
		.with_context(|| format!("creating {}", dir.display()))?;
	// The directory itself, every `..` and link resolved: what the files will
	// really be written under.
	let real = fs::canonicalize(&dir).with_context(|| format!("resolving {}", dir.display()))?;
	let mounts = fs::read_to_string("/proc/self/mounts").context("reading /proc/self/mounts to see what holds the decrypted files")?;
	match filesystem(&mounts, &real) {
		Some("tmpfs" | "ramfs") => Ok(real),
		Some(other) => bail!("{} is on {other}, not tmpfs: a decrypted value is never written to a disk", real.display()),
		None => bail!("no mount holds {}", real.display()),
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

/// The decrypted files of one run, in a directory of its own; the directory
/// goes when this is dropped.
///
/// Its own, so runs never touch each other's files: a `make compose ARGS="logs
/// -f …"` ending while a `make up` starts, or two roles at once, would
/// otherwise remove or replace the other's before compose has read them.
#[derive(Debug)]
pub(crate) struct Delivered {
	dir: PathBuf,
}

impl Delivered {
	/// `<base>/<pid>/<app>.env` for each app: mode 600 in a mode 700
	/// directory that only this run uses. A run's directory whose process is
	/// gone (one killed with SIGKILL) is removed first, and so is a file an
	/// older envctl left directly in `base`.
	pub(crate) fn write(base: &Path, apps: &IndexMap<String, Plain>) -> Result<Self> {
		fs::DirBuilder::new()
			.recursive(true)
			.mode(0o700)
			.create(base)
			.with_context(|| format!("creating {}", base.display()))?;
		let own = std::process::id();
		for entry in fs::read_dir(base).with_context(|| format!("reading {}", base.display()))?.filter_map(Result::ok) {
			let path = entry.path();
			let name = entry.file_name().to_string_lossy().into_owned();
			let older_file = path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("env"));
			match name.parse::<u32>() {
				Ok(pid) if pid == own || !Path::new("/proc").join(&name).exists() => {
					let _ = fs::remove_dir_all(&path);
				}
				Err(_) if older_file => {
					let _ = fs::remove_file(&path);
				}
				_ => {}
			}
		}
		let dir = base.join(own.to_string());
		fs::DirBuilder::new().mode(0o700).create(&dir).with_context(|| format!("creating {}", dir.display()))?;
		let delivered = Self { dir };
		for (app, entries) in apps {
			let path = delivered.dir.join(format!("{app}.env"));
			let mut text = String::new();
			for (name, value) in entries {
				text.push_str(&compose_line(name, value)?);
			}
			let mut file = OpenOptions::new()
				.write(true)
				.create_new(true)
				.mode(0o600)
				.open(&path)
				.with_context(|| format!("creating {}", path.display()))?;
			file.write_all(text.as_bytes()).with_context(|| format!("writing {}", path.display()))?;
		}
		Ok(delivered)
	}

	/// This run's directory: compose's `ENV_DIR`.
	pub(crate) fn dir(&self) -> &Path {
		&self.dir
	}
}

impl Drop for Delivered {
	fn drop(&mut self) {
		let _ = fs::remove_dir_all(&self.dir);
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
	fn each_run_has_its_own_private_directory_gone_after() {
		let base = std::env::temp_dir().join(format!("envctl-deliver-{}", std::process::id()));
		let apps = IndexMap::from([("probe".to_owned(), Plain::from([("A".to_owned(), "1".to_owned())]))]);
		std::fs::create_dir_all(base.join("4294967295")).unwrap();
		std::fs::create_dir_all(base.join("1")).unwrap();
		std::fs::write(base.join("probe.env"), "STALE=1\n").unwrap();
		{
			let delivered = Delivered::write(&base, &apps).unwrap();
			let path = delivered.dir().join("probe.env");
			assert_eq!(delivered.dir(), base.join(std::process::id().to_string()));
			assert_eq!(std::fs::read_to_string(&path).unwrap(), "A='1'\n");
			assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
			assert_eq!(std::fs::metadata(delivered.dir()).unwrap().permissions().mode() & 0o777, 0o700);
			assert!(!base.join("4294967295").exists(), "a dead run's directory is swept");
			assert!(base.join("1").exists(), "a live process's directory is not touched");
			assert!(!base.join("probe.env").exists(), "an older envctl's file is swept");
			drop(delivered);
			assert!(!path.exists() && !base.join(std::process::id().to_string()).exists());
		}
		std::fs::remove_dir_all(&base).unwrap();
	}
}
