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

	// Check if Dockerfile changed
	let dockerfile = normalize_path(Path::new(image.dockerfile), workspace_root);
	for file in changed_files {
		let normalized_file = normalize_path(file, workspace_root);
		if normalized_file == dockerfile {
			eprintln!("  ✓ Dockerfile changed: {}", image.dockerfile);
			return true;
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
