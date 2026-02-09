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
	migration_paths: Option<&'static [&'static str]>,

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
		migration_paths: Some(&["apps/servers/file_host/migrations"]),
		manifest: "apps/servers/file_host/Cargo.toml",
	},
	ImageSpec {
		name: "some-obs",
		dockerfile: "./infra/docker/Dockerfile.obs",
		repo_suffix: "obs",
		needs_sqlx: false,
		needs_migrations: false,
		migration_paths: None,
		manifest: "apps/some-obs/Cargo.toml",
	},
	ImageSpec {
		name: "orchestrator",
		dockerfile: "./infra/docker/Dockerfile.orchestrator",
		repo_suffix: "orchestrator",
		needs_sqlx: false,
		needs_migrations: false,
		migration_paths: None,
		manifest: "apps/orchestrator/Cargo.toml",
	},
];

/* ------------------------- METADATA TYPES ------------------------- */

#[derive(Deserialize)]
struct Metadata {
	packages: Vec<Package>,
	resolve: Resolve,
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
		panic!("cargo metadata failed");
	}

	serde_json::from_slice(&output.stdout).expect("invalid metadata json")
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
	let dockerfile = Path::new(image.dockerfile);

	if changed_files.iter().any(|f| f == dockerfile) {
		return true;
	}

	// map manifest path → package id
	let pkg = metadata.packages.iter().find(|p| p.manifest_path.ends_with(image.manifest));

	let pkg = match pkg {
		Some(p) => p,
		None => return false,
	};

	let closure = dependency_closure(&pkg.id, graph);

	// build package id → crate root dir
	let mut crate_dirs = HashSet::new();

	for p in &metadata.packages {
		if closure.contains(&p.id) {
			if let Some(parent) = Path::new(&p.manifest_path).parent() {
				crate_dirs.insert(parent.to_path_buf());
			}
		}
	}

	for file in changed_files {
		if crate_dirs.iter().any(|dir| file.starts_with(dir)) {
			return true;
		}
	}

	false
}

/* ------------------------- MAIN ------------------------- */

fn main() {
	let force = env::var("FORCE_BUILD").is_ok();

	let changed_files: Vec<PathBuf> = io::stdin()
		.lock()
		.lines()
		.filter_map(Result::ok)
		.map(|l| l.trim().to_string())
		.filter(|l| !l.is_empty())
		.map(PathBuf::from)
		.collect();

	let metadata = load_metadata();
	let graph = build_graph(&metadata);

	let mut include = Vec::new();

	for image in IMAGES {
		if force || needs_rebuild(image, &changed_files, &metadata, &graph) {
			include.push(image.clone());
		}
	}

	let matrix = Matrix { include };

	println!("{}", serde_json::to_string(&matrix).unwrap());
}
