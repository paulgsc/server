//! Detects if a Rust app needs rebuilding based on:
//! 1. Changes in its own directory
//! 2. Changes in any crate it depends on (via cargo tree)
//! 3. Changes to its Dockerfile

use std::collections::HashSet;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn get_dependencies(manifest_path: &Path) -> HashSet<String> {
	let output = Command::new("cargo")
		.args(["tree", "--manifest-path", manifest_path.to_str().unwrap(), "--prefix", "none", "--edges", "normal"])
		.output()
		.expect("Failed to run cargo tree");

	if !output.status.success() {
		eprintln!("⚠️ Warning: cargo tree failed for {}", manifest_path.display());
		return HashSet::new();
	}

	let stdout = String::from_utf8_lossy(&output.stdout);
	stdout.lines().filter_map(|line| line.split_whitespace().next().map(|s| s.to_string())).collect()
}

/// Parse crate name from Cargo.toml
fn get_crate_name(manifest_path: &Path) -> Option<String> {
	let toml_str = std::fs::read_to_string(manifest_path).ok()?;

	// Simple TOML parsing without external dependencies
	for line in toml_str.lines() {
		let trimmed = line.trim();
		if trimmed.starts_with("name") && trimmed.contains('=') {
			if let Some(name_part) = trimmed.split('=').nth(1) {
				let name = name_part.trim().trim_matches('"').trim_matches('\'');
				return Some(name.to_string());
			}
		}
	}
	None
}

/// Normalize crate name to handle hyphens/underscores
fn normalize_crate_name(name: &str) -> HashSet<String> {
	let mut variants = HashSet::new();
	variants.insert(name.to_string());
	variants.insert(name.replace('-', "_"));
	variants.insert(name.replace('_', "-"));
	variants
}

fn check_changes(app_dir: &Path, manifest_path: &Path, dockerfile_path: &Path, changed_files: &[PathBuf]) -> (bool, HashSet<String>) {
	// Dependencies + the crate itself
	let mut deps = get_dependencies(manifest_path);
	if let Some(crate_name) = get_crate_name(manifest_path) {
		deps.insert(crate_name);
	}

	let app_dir = app_dir.canonicalize().unwrap();

	for file in changed_files {
		let file = file.canonicalize().unwrap_or_else(|_| file.clone());

		// 1. Check app directory
		if file.starts_with(&app_dir) {
			println!("✓ App directory changed: {}", file.display());
			return (true, deps);
		}

		// 2. Check Dockerfile
		if file == dockerfile_path.canonicalize().unwrap() {
			println!("✓ Dockerfile changed: {}", file.display());
			return (true, deps);
		}

		// 3. Check dependencies (assumes crates/CRATE_NAME/... layout)
		if let Some(crate_dir) = file.iter().nth(1) {
			let crate_dir_str = crate_dir.to_string_lossy();
			let normalized = normalize_crate_name(&crate_dir_str);
			if deps.intersection(&normalized).next().is_some() {
				println!("✓ Dependency crate changed: {} (matches dep)", crate_dir_str);
				return (true, deps);
			}
		}
	}

	println!("✗ No relevant changes detected");
	(false, deps)
}

fn main() {
	let args: Vec<String> = env::args().collect();
	if args.len() != 5 {
		eprintln!(
			"Usage: {} <app_dir> <manifest_path> <dockerfile_path> <changed_files>",
			args.first().unwrap_or(&"detect_crate_changes".to_string())
		);
		std::process::exit(1);
	}

	let app_dir = Path::new(&args[1]);
	let manifest_path = Path::new(&args[2]);
	let dockerfile_path = Path::new(&args[3]);
	let changed_files_raw = &args[4];

	let changed_files: Vec<PathBuf> = changed_files_raw.lines().map(|s| PathBuf::from(s.trim())).filter(|s| !s.as_os_str().is_empty()).collect();

	println!("\n--- Checking {} ---", app_dir.display());
	let (changed, deps) = check_changes(app_dir, manifest_path, dockerfile_path, &changed_files);

	let dep_sample: Vec<&String> = deps.iter().take(10).collect();
	println!(
		"Dependencies ({}): {}{}",
		deps.len(),
		dep_sample.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "),
		if deps.len() > 10 { "..." } else { "" }
	);

	println!("Result: {}\n", if changed { "REBUILD" } else { "SKIP" });

	// Output for GitHub Actions
	println!("changed={}", if changed { "true" } else { "false" });
}
