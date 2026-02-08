//! Detects if a Rust app needs rebuilding based on:
//! 1. Changes in its own directory
//! 2. Changes in any crate it depends on (via cargo tree)
//! 3. Changes to its Dockerfile

use std::collections::HashSet;
use std::env;
use std::fs;
use std::process::Command;

fn get_dependencies(manifest_path: &str) -> HashSet<String> {
    let output = Command::new("cargo")
        .args([
            "tree",
            "--manifest-path",
            manifest_path,
            "--prefix",
            "none",
            "--edges",
            "normal",
        ])
        .output()
        .expect("Failed to run cargo tree");

    if !output.status.success() {
        eprintln!("⚠️  Warning: cargo tree failed for {}", manifest_path);
        return HashSet::new();
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .filter_map(|line| {
            // cargo tree format: "crate-name v0.1.0 (/path/to/crate)"
            let parts: Vec<&str> = line.split_whitespace().collect();
            parts.first().map(|s| s.to_string())
        })
        .collect()
}

fn normalize_crate_name(name: &str) -> HashSet<String> {
    // Handle crate naming inconsistencies (hyphens vs underscores)
    let mut variants = HashSet::new();
    variants.insert(name.to_string());
    variants.insert(name.replace('-', "_"));
    variants.insert(name.replace('_', "-"));
    variants
}

fn check_changes(
    app_dir: &str,
    manifest_path: &str,
    dockerfile_path: &str,
    changed_files: &[String],
) -> (bool, HashSet<String>) {
    let deps = get_dependencies(manifest_path);
    let app_dir_normalized = app_dir.trim_end_matches('/');

    // 1. Check if app's own directory changed
    for file in changed_files {
        if file.starts_with(&format!("{}/", app_dir_normalized)) {
            println!("✓ App directory changed: {}", file);
            return (true, deps);
        }
    }

    // 2. Check if any dependency crate changed
    for file in changed_files {
        if file.starts_with("crates/") {
            if let Some(crate_dir) = file.split('/').nth(1) {
                let normalized = normalize_crate_name(crate_dir);
                for variant in &normalized {
                    if deps.contains(variant) {
                        println!(
                            "✓ Dependency crate changed: {} (matches {})",
                            crate_dir, variant
                        );
                        return (true, deps);
                    }
                }
            }
        }
    }

    // 3. Check if Dockerfile changed
    for file in changed_files {
        if file == dockerfile_path {
            println!("✓ Dockerfile changed: {}", file);
            return (true, deps);
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

    let app_dir = &args[1];
    let manifest_path = &args[2];
    let dockerfile_path = &args[3];
    let changed_files_raw = &args[4];

    let changed_files: Vec<String> = changed_files_raw
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    println!("\n--- Checking {} ---", app_dir);

    let (changed, deps) = check_changes(app_dir, manifest_path, dockerfile_path, &changed_files);

    let dep_sample: Vec<&String> = deps.iter().take(10).collect();
    println!(
        "Dependencies ({}): {}{}",
        deps.len(),
        dep_sample
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        if deps.len() > 10 { "..." } else { "" }
    );
    println!("Result: {}\n", if changed { "REBUILD" } else { "SKIP" });

    // Output for GitHub Actions
    println!("changed={}", if changed { "true" } else { "false" });
}
