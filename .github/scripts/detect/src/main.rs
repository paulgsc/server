//! Emits GitHub Actions matrix:
//! { "include": [ ... ] }
//!
//! Reads changed file paths from STDIN (one per line)

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::env;
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Serialize, Clone)]
struct ImageSpec {
	name: &'static str,
	dockerfile: &'static str,
	repo_suffix: &'static str,
	needs_sqlx: bool,
	needs_migrations: bool,

	#[serde(skip_serializing)]
	manifest: &'static str,
}

#[derive(Serialize)]
struct Matrix {
	include: Vec<ImageSpec>,
}

/* ------------------------- IMAGE CONFIG ------------------------- */

const IMAGES: &[ImageSpec] = &[
	ImageSpec {
		name: "file_host",
		dockerfile: "./infra/docker/Dockerfile.server",
		repo_suffix: "server",
		needs_sqlx: true,
		needs_migrations: true,
		manifest: "apps/servers/file_host/Cargo.toml",
	},
	ImageSpec {
		name: "maishatu-obs",
		dockerfile: "./infra/docker/Dockerfile.obs",
		repo_suffix: "obs",
		needs_sqlx: false,
		needs_migrations: false,
		manifest: "apps/some-obs/Cargo.toml",
	},
	ImageSpec {
		name: "orchestrator",
		dockerfile: "./infra/docker/Dockerfile.orchestrator",
		repo_suffix: "orchestrator",
		needs_sqlx: false,
		needs_migrations: false,
		manifest: "apps/orchestrator/Cargo.toml",
	},
];

/* ------------------------- METADATA TYPES ------------------------- */

#[derive(Deserialize)]
struct Metadata {
	packages: Vec<Package>,
	resolve: Resolve,
	workspace_root: String,
}

#[derive(Deserialize)]
struct Package {
	id: String,
	manifest_path: String,
}

#[derive(Deserialize)]
struct Resolve {
	nodes: Vec<Node>,
}

#[derive(Deserialize)]
struct Node {
	id: String,
	dependencies: Vec<String>,
}

/* ------------------------- METADATA LOADER ------------------------- */

fn load_metadata() -> Metadata {
	let output = Command::new("cargo")
		.args(["metadata", "--format-version", "1", "--locked"])
		.output()
		.expect("failed to run cargo metadata");

	if !output.status.success() {
		eprintln!("cargo metadata stderr: {}", String::from_utf8_lossy(&output.stderr));
		panic!("cargo metadata failed");
	}

	serde_json::from_slice(&output.stdout).expect("invalid metadata json")
}

/* ------------------------- PATH UTILITIES ------------------------- */

/// Convert to absolute path relative to workspace root
fn normalize_path(path: &Path, workspace_root: &Path) -> PathBuf {
	if path.is_absolute() {
		path.to_path_buf()
	} else {
		workspace_root.join(path)
	}
}

/// Check if file is under directory (handles both relative and absolute)
fn is_under_dir(file: &Path, dir: &Path) -> bool {
	// Try direct prefix check
	if file.starts_with(dir) {
		return true;
	}

	// Try canonicalized paths (handles symlinks, .., etc)
	match (file.canonicalize(), dir.canonicalize()) {
		(Ok(file_canon), Ok(dir_canon)) => file_canon.starts_with(dir_canon),
		_ => false,
	}
}

/// Check whether a changed file affects the image build/publish pipeline.
fn is_build_pipeline_file(file: &Path, workspace_root: &Path) -> bool {
	const PIPELINE_FILES: &[&str] = &[
		".github/workflows/detect.yml",
		".github/workflows/build-image.yml",
		".github/workflows/push-image.yml",
		".github/workflows/publish-image-changesets.yml",
	];
	const PIPELINE_DIRS: &[&str] = &[".github/actions/docker-build", ".github/actions/docker-push", ".github/scripts/detect"];

	PIPELINE_FILES.iter().map(|path| normalize_path(Path::new(path), workspace_root)).any(|path| file == path)
		|| PIPELINE_DIRS
			.iter()
			.map(|path| normalize_path(Path::new(path), workspace_root))
			.any(|path| is_under_dir(file, &path))
}

/// Check whether a path belongs to the image's application crate.
///
/// This check deliberately uses the configured manifest path instead of the
/// Cargo dependency graph. In particular, deleted source files cannot be
/// canonicalized, but they still have to rebuild the image which previously
/// contained them.
fn is_image_crate_file(image: &ImageSpec, file: &Path, workspace_root: &Path) -> bool {
	let manifest = normalize_path(Path::new(image.manifest), workspace_root);
	let crate_dir = manifest.parent().expect("image manifest must have a parent directory");
	is_under_dir(file, crate_dir)
}

/* ------------------------- DEP GRAPH ------------------------- */

fn build_graph(metadata: &Metadata) -> HashMap<String, HashSet<String>> {
	let mut graph = HashMap::new();

	for node in &metadata.resolve.nodes {
		graph.insert(node.id.clone(), node.dependencies.iter().cloned().collect());
	}

	graph
}

fn dependency_closure(root: &str, graph: &HashMap<String, HashSet<String>>) -> HashSet<String> {
	let mut visited = HashSet::new();
	let mut stack = vec![root.to_string()];

	while let Some(current) = stack.pop() {
		if visited.insert(current.clone()) {
			if let Some(deps) = graph.get(&current) {
				for dep in deps {
					stack.push(dep.clone());
				}
			}
		}
	}

	visited
}

/* ------------------------- REBUILD LOGIC ------------------------- */

fn needs_rebuild(image: &ImageSpec, changed_files: &[PathBuf], metadata: &Metadata, graph: &HashMap<String, HashSet<String>>) -> bool {
	let workspace_root = Path::new(&metadata.workspace_root);

	eprintln!("Checking image: {}", image.name);

	// An image always owns its application crate. Check that stable boundary
	// before consulting Cargo metadata so source deletions and dependency
	// removals cannot disappear from the post-change dependency graph.
	for file in changed_files {
		let normalized_file = normalize_path(file, workspace_root);
		if is_image_crate_file(image, &normalized_file, workspace_root) {
			eprintln!("  ✓ Changed file in image crate: {}", file.display());
			return true;
		}
	}

	// Changes to the shared build/publish pipeline can alter the output or
	// publishing behavior of every image. Rebuild immediately so a CI-only
	// fix is rolled out without waiting for an unrelated application change.
	for file in changed_files {
		let normalized_file = normalize_path(file, workspace_root);
		if is_build_pipeline_file(&normalized_file, workspace_root) {
			eprintln!("  ✓ Image pipeline changed: {}", file.display());
			return true;
		}
	}

	// SQLx preparation is only used by images which opt in to SQLx metadata.
	if image.needs_sqlx {
		let prepare_sqlx = normalize_path(Path::new(".github/actions/prepare-sqlx"), workspace_root);
		for file in changed_files {
			let normalized_file = normalize_path(file, workspace_root);
			if is_under_dir(&normalized_file, &prepare_sqlx) {
				eprintln!("  ✓ SQLx preparation changed: {}", file.display());
				return true;
			}
		}
	}

	// Check if Dockerfile changed
	let dockerfile = normalize_path(Path::new(image.dockerfile), workspace_root);
	for file in changed_files {
		let normalized_file = normalize_path(file, workspace_root);
		if normalized_file == dockerfile {
			eprintln!("  ✓ Dockerfile changed: {}", image.dockerfile);
			return true;
		}
	}

	// The server's schema is owned by the hoisted workspace migrations
	// directory rather than by an individual crate in its dependency graph.
	if image.needs_migrations {
		let migrations = normalize_path(Path::new("migrations"), workspace_root);
		for file in changed_files {
			let normalized_file = normalize_path(file, workspace_root);
			if is_under_dir(&normalized_file, &migrations) {
				eprintln!("  ✓ Workspace migration changed: {}", file.display());
				return true;
			}
		}
	}

	// Find the package by manifest path
	let pkg = metadata.packages.iter().find(|p| {
		let pkg_manifest = Path::new(&p.manifest_path);
		let expected_manifest = normalize_path(Path::new(image.manifest), workspace_root);
		pkg_manifest == expected_manifest
	});

	let pkg = match pkg {
		Some(p) => {
			eprintln!("  Found package: {}", p.id);
			p
		}
		None => {
			eprintln!("  ✗ Package not found for manifest: {}", image.manifest);
			return false;
		}
	};

	// Get all dependencies
	let closure = dependency_closure(&pkg.id, graph);
	eprintln!("  Dependency closure size: {}", closure.len());

	// Build set of all crate directories in the dependency tree
	let mut crate_dirs = HashSet::new();

	for p in &metadata.packages {
		if closure.contains(&p.id) {
			if let Some(parent) = Path::new(&p.manifest_path).parent() {
				crate_dirs.insert(parent.to_path_buf());
			}
		}
	}

	eprintln!("  Watching {} crate directories", crate_dirs.len());

	// Check if any changed file is in a relevant crate
	for file in changed_files {
		let normalized_file = normalize_path(file, workspace_root);

		for dir in &crate_dirs {
			if is_under_dir(&normalized_file, dir) {
				eprintln!("  ✓ Changed file in dependency: {}", file.display());
				eprintln!("    (matches crate: {})", dir.display());
				return true;
			}
		}
	}

	eprintln!("  ✗ No relevant changes detected");
	false
}

/* ------------------------- MAIN ------------------------- */

fn main() {
	let force = env::var("FORCE_BUILD").is_ok();

	eprintln!("=== Docker Build Matrix Generator ===");
	eprintln!("FORCE_BUILD: {}", force);

	let changed_files: Vec<PathBuf> = io::stdin()
		.lock()
		.lines()
		.filter_map(Result::ok)
		.map(|l| l.trim().to_string())
		.filter(|l| !l.is_empty())
		.map(PathBuf::from)
		.collect();

	eprintln!("Changed files ({}):", changed_files.len());
	for file in &changed_files {
		eprintln!("  - {}", file.display());
	}

	let metadata = load_metadata();
	eprintln!("Workspace root: {}", metadata.workspace_root);

	let graph = build_graph(&metadata);

	let mut include = Vec::new();

	for image in IMAGES {
		if force || needs_rebuild(image, &changed_files, &metadata, &graph) {
			eprintln!("➜ REBUILDING: {}", image.name);
			include.push(image.clone());
		}
	}

	let matrix = Matrix { include: include.clone() };

	eprintln!("\n=== Final Matrix ===");
	eprintln!("Images to build: {}", include.len());

	println!("{}", serde_json::to_string(&matrix).unwrap());
}

#[cfg(test)]
mod tests {
	use super::{is_image_crate_file, normalize_path, IMAGES};
	use std::path::Path;

	#[test]
	fn deleted_file_in_image_crate_is_detected() {
		let workspace_root = Path::new("/tmp/detect-workspace");
		let deleted_file = normalize_path(Path::new("apps/servers/file_host/src/routes/sheets.rs"), workspace_root);

		assert!(is_image_crate_file(&IMAGES[0], &deleted_file, workspace_root));
	}

	#[test]
	fn another_images_crate_is_not_detected() {
		let workspace_root = Path::new("/tmp/detect-workspace");
		let obs_file = normalize_path(Path::new("apps/some-obs/src/main.rs"), workspace_root);

		assert!(!is_image_crate_file(&IMAGES[0], &obs_file, workspace_root));
	}
}
