/// Store — responsible for reading and writing the registry file.
///
/// This module owns all I/O.  It knows nothing about:
///   - CLI argument parsing
///   - fuzzy matching
///   - picker UX
///
/// It receives and returns domain types only.
///
/// # Filesystem scope
///
/// The module is intentionally sandboxed: all I/O is constrained to a single
/// file whose location is governed by [`RegistryPath`].  No other path on the
/// host filesystem is ever read, written, or deleted.
///
/// `RegistryPath` is an opaque, validated newtype — it cannot be constructed
/// from an arbitrary `&Path` by callers.  Resolution happens through
/// [`RegistryPath::resolve`], which applies the precedence rules and enforces
/// the structural constraints.  If the resolved path does not satisfy those
/// constraints, construction fails with a typed error; no I/O occurs.
///
/// # Nix shell integration
///
/// The recommended pattern for per-repo scoping is to set `POCKET_REGISTRY`
/// in the repository's `shellHook`:
///
/// ```nix
/// shellHook = ''
///   export POCKET_REGISTRY="$PWD/.pocket/registers.json"
/// '';
/// ```
///
/// `RegistryPath::resolve` will validate this value just like any other
/// source, so a misconfigured `shellHook` produces a clear error rather than
/// silently writing to an unintended location.
use std::path::{Path, PathBuf};

use atomicwrites::{AtomicFile, OverwriteBehavior};
use serde_json;

use crate::{
	error::{PocketError, Result},
	model::{Checked, Registry, RegistryData, Unchecked},
};

pub const DEFAULT_MAX_ENTRIES: usize = 128;
pub const REGISTRY_FILE_NAME: &str = "registers.json";
pub const REGISTRY_DIR_NAME: &str = "pocket_registry";
pub const REGISTRY_DIR_NAME_HIDDEN: &str = ".pocket_registry";

// ── RegistryPath ──────────────────────────────────────────────────────────────

/// An opaque, validated handle to the single registry file this process may
/// read or write.
///
/// # Invariants (enforced at construction)
///
/// 1. The file name component is exactly [`REGISTRY_FILE_NAME`].
/// 2. The immediate parent directory is named [`REGISTRY_DIR_NAME`] or
///    [`REGISTRY_DIR_NAME_HIDDEN`] — this constrains the tool to its own
///    namespace and prevents it from being aimed at an arbitrary directory.
/// 3. The path contains no `..` components (no traversal out of the intended
///    subtree).
///
/// These invariants are sufficient to guarantee that even a misconfigured
/// `POCKET_REGISTRY` env var — e.g. one pointing at `~/important/data.json`
/// or `/etc/passwd` — causes a hard error at resolution time, before any I/O.
///
/// The only public constructor is [`RegistryPath::resolve`].
#[derive(Debug, Clone)]
pub struct RegistryPath(PathBuf);

impl RegistryPath {
	/// Resolve and validate the registry path.
	///
	/// Precedence:
	///   1. `POCKET_REGISTRY` env var (Nix `shellHook` / power users)
	///   2. `$XDG_CONFIG_HOME/pocket/registers.json`
	///   3. `~/.config/pocket/registers.json`
	///
	/// Returns [`PocketError::InvalidRegistryPath`] if the resolved path
	/// violates the structural invariants — no I/O is attempted in that case.
	pub fn resolve() -> Result<Self> {
		let path = if let Ok(env) = std::env::var("POCKET_REGISTRY") {
			PathBuf::from(env)
		} else {
			let base = std::env::var("XDG_CONFIG_HOME")
				.map(PathBuf::from)
				.unwrap_or_else(|_| dirs_next::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".config"));
			base.join(REGISTRY_DIR_NAME).join(REGISTRY_FILE_NAME)
		};

		Self::validate(path)
	}

	/// Validate a candidate path and wrap it if all invariants hold.
	fn validate(path: PathBuf) -> Result<Self> {
		// Invariant 1: no `..` components — reject traversal attempts.
		if path.components().any(|c| c == std::path::Component::ParentDir) {
			return Err(PocketError::InvalidRegistryPath {
				path: path.to_path_buf(),
				reason: "path must not contain `..` components".into(),
			});
		}

		// Invariant 2: file name must be exactly REGISTRY_FILE_NAME.
		match path.file_name().and_then(|n| n.to_str()) {
			Some(name) if name == REGISTRY_FILE_NAME => {}
			Some(name) => {
				return Err(PocketError::InvalidRegistryPath {
					path: path.to_path_buf(),
					reason: format!("file name must be `{REGISTRY_FILE_NAME}`, got `{name}`"),
				})
			}
			None => {
				return Err(PocketError::InvalidRegistryPath {
					path,
					reason: "path has no file name component".into(),
				})
			}
		}

		// Invariant 3: immediate parent directory must be `pocket` or `.pocket`.
		let parent_name = path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str());

		match parent_name {
			Some(dir) if dir == REGISTRY_DIR_NAME || dir == REGISTRY_DIR_NAME_HIDDEN => {}
			Some(dir) => {
				return Err(PocketError::InvalidRegistryPath {
					path: path.to_path_buf(),
					reason: format!(
						"parent directory must be `{REGISTRY_DIR_NAME}` or \
                         `{REGISTRY_DIR_NAME_HIDDEN}`, got `{dir}`"
					),
				})
			}
			None => {
				return Err(PocketError::InvalidRegistryPath {
					path,
					reason: "path has no parent directory component".into(),
				})
			}
		}

		Ok(Self(path))
	}

	/// Borrow the underlying path.
	///
	/// `pub(crate)` — external callers have no need to escape the abstraction.
	pub(crate) fn as_path(&self) -> &Path {
		&self.0
	}
}

impl std::fmt::Display for RegistryPath {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		self.0.display().fmt(f)
	}
}

// ── Internal helpers ───────────────────────────────────────────────────────────

/// Ensure the parent directory of `path` exists.
///
/// Private — called only during `save`; `RegistryPath` guarantees we are
/// within our own namespace before this ever runs.
fn ensure_parent(path: &Path) -> Result<()> {
	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent).map_err(|source| PocketError::ConfigDirCreate {
			path: parent.to_path_buf(),
			source,
		})?;
	}
	Ok(())
}

// ── Public I/O surface ─────────────────────────────────────────────────────────

/// Load the registry from `path`.
///
/// Returns [`PocketError::RegistryNotFound`] if the file does not yet exist —
/// callers may choose to initialise a fresh registry in that case.
pub fn load(path: &RegistryPath) -> Result<Registry<Unchecked>> {
	let p = path.as_path();
	let raw = std::fs::read(p).map_err(|source| {
		if source.kind() == std::io::ErrorKind::NotFound {
			PocketError::RegistryNotFound { path: p.to_path_buf() }
		} else {
			PocketError::RegistryRead { path: p.to_path_buf(), source }
		}
	})?;

	let data: RegistryData = serde_json::from_slice(&raw).map_err(|source| PocketError::Deserialize { source })?;

	Ok(Registry::from_data(data))
}

/// Load the registry, or create a fresh one if absent.
///
/// The returned registry has passed invariant checks.
pub fn load_or_init(path: &RegistryPath) -> Result<Registry<Checked>> {
	match load(path) {
		Ok(unchecked) => unchecked.check(),

		Err(PocketError::RegistryNotFound { .. }) => {
			let fresh = Registry::<Unchecked>::new(DEFAULT_MAX_ENTRIES)
				.check()
				.expect("empty registry with positive max must be valid");
			Ok(fresh)
		}

		Err(e) => Err(e),
	}
}

/// Persist a checked registry atomically.
///
/// Uses `AtomicFile` so a crash mid-write cannot corrupt the registry.
/// The destination is guaranteed by `RegistryPath` to be within the tool's
/// own namespace — this function will never touch any other file.
pub fn save(registry: &Registry<Checked>, path: &RegistryPath) -> Result<()> {
	let p = path.as_path();
	ensure_parent(p)?;

	let json = serde_json::to_vec_pretty(registry.as_data()).map_err(|source| PocketError::Serialize { source })?;

	AtomicFile::new(p, OverwriteBehavior::AllowOverwrite)
		.write(|f| {
			use std::io::Write;
			f.write_all(&json)
		})
		.map_err(|e| PocketError::RegistryWrite {
			path: p.to_path_buf(),
			source: std::io::Error::new(std::io::ErrorKind::Other, e.to_string()),
		})?;

	Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
	use super::*;
	use tempfile::TempDir;

	// Construct a RegistryPath that points into a TempDir.
	// Uses the hidden variant to exercise both accepted dir names.
	fn temp_registry_path(dir: &TempDir) -> RegistryPath {
		let path = dir.path().join(REGISTRY_DIR_NAME_HIDDEN).join(REGISTRY_FILE_NAME);
		RegistryPath::validate(path).expect("temp path must be valid")
	}

	// ── RegistryPath validation ────────────────────────────────────────────────

	#[test]
	fn accepts_pocket_dir() {
		let path = PathBuf::from("/home/user/.config/pocket/registers.json");
		assert!(RegistryPath::validate(path).is_ok());
	}

	#[test]
	fn accepts_hidden_pocket_dir() {
		let path = PathBuf::from("/home/user/projects/repo/.pocket/registers.json");
		assert!(RegistryPath::validate(path).is_ok());
	}

	#[test]
	fn rejects_wrong_file_name() {
		let path = PathBuf::from("/home/user/.pocket/Cargo.toml");
		let err = RegistryPath::validate(path).unwrap_err();
		assert!(matches!(err, PocketError::InvalidRegistryPath { .. }));
	}

	#[test]
	fn rejects_wrong_parent_dir() {
		// Points at the right filename but in a wrong directory.
		let path = PathBuf::from("/home/user/important_stuff/registers.json");
		let err = RegistryPath::validate(path).unwrap_err();
		assert!(matches!(err, PocketError::InvalidRegistryPath { .. }));
	}

	#[test]
	fn rejects_path_traversal() {
		let path = PathBuf::from("/home/user/.pocket/../etc/passwd");
		let err = RegistryPath::validate(path).unwrap_err();
		assert!(matches!(err, PocketError::InvalidRegistryPath { .. }));
	}

	#[test]
	fn rejects_arbitrary_json_file() {
		// Simulates someone accidentally pointing POCKET_REGISTRY at package.json.
		let path = PathBuf::from("/home/user/project/package.json");
		let err = RegistryPath::validate(path).unwrap_err();
		assert!(matches!(err, PocketError::InvalidRegistryPath { .. }));
	}

	// ── I/O ───────────────────────────────────────────────────────────────────

	#[test]
	fn round_trip_empty_registry() {
		let dir = TempDir::new().unwrap();
		let path = temp_registry_path(&dir);

		let registry = load_or_init(&path).unwrap();
		save(&registry, &path).unwrap();

		let reloaded = load_or_init(&path).unwrap();
		assert_eq!(reloaded.len(), 0);
		assert_eq!(reloaded.max_entries(), DEFAULT_MAX_ENTRIES);
	}

	#[test]
	fn missing_file_returns_fresh_registry_not_error() {
		let dir = TempDir::new().unwrap();
		let path = temp_registry_path(&dir);
		let r = load_or_init(&path).unwrap();
		assert!(r.is_empty());
	}

	#[test]
	fn load_corrupted_json_returns_deserialize_error() {
		let dir = TempDir::new().unwrap();
		let path = temp_registry_path(&dir);
		// Create the parent dir so the write succeeds.
		std::fs::create_dir_all(path.as_path().parent().unwrap()).unwrap();
		std::fs::write(path.as_path(), b"not json at all {{{{").unwrap();
		let err = load(&path).unwrap_err();
		assert!(matches!(err, PocketError::Deserialize { .. }));
	}

	#[test]
	fn save_is_atomic_on_success() {
		let dir = TempDir::new().unwrap();
		let path = temp_registry_path(&dir);
		let registry = load_or_init(&path).unwrap();
		save(&registry, &path).unwrap();
		let _ = load_or_init(&path).unwrap();
	}
}
