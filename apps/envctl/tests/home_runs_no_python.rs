//! Nothing home runs calls python: home (NixOS) has no `python3` on PATH,
//! and `make home-deploy` once failed there with `make: python3: No such
//! file or directory` because the deploy was a python script (server#PRNUM).
//! Home's commands are the Makefile's targets and the NixOS modules' units,
//! and whatever under `scripts/` they name; this holds all of them to it.
//! CI's own checks under `scripts/` run on GitHub's runners and stay python,
//! so a script is held only when the Makefile or a module names it.
//! `nix/modules/test.nix` is evaluated in CI, not on home, so it is not
//! held either; it asserts the deploy unit's PATH and commands itself.

// A test that reads files and names what it found: the formatting macros are
// the plain way to say it, and nothing here is a hot path.
#![allow(clippy::disallowed_macros)]

use regex::Regex;
use std::path::{Path, PathBuf};

/// `python`, `python3`, `python3.12`, `pkgs.python3`, `python3Packages`: a
/// word starting `python`.
fn python() -> Regex {
	Regex::new(r"(?i)\bpython").unwrap()
}

/// `scripts/<file>` as a recipe or a unit names it.
fn script_ref() -> Regex {
	Regex::new(r"\bscripts/[A-Za-z0-9_./-]+").unwrap()
}

/// Lines a shell, make or nix would run: not a comment.
fn code_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
	text
		.lines()
		.enumerate()
		.filter(|(_, line)| !line.trim_start().starts_with('#'))
		.map(|(at, line)| (at + 1, line))
}

/// Every place `text` (from `shown`) calls python or names a python script,
/// and the scripts it names, to be read the same way.
fn findings(shown: &str, text: &str, found: &mut Vec<String>, named: &mut Vec<String>) {
	if text.lines().next().is_some_and(|first| first.starts_with("#!") && python().is_match(first)) {
		found.push(format!("{shown}:1: its interpreter is python"));
	}
	for (at, line) in code_lines(text) {
		if python().is_match(line) {
			found.push(format!("{shown}:{at}: {}", line.trim()));
		}
		for script in script_ref().find_iter(line) {
			let script = script.as_str().trim_end_matches(['.', '/']).to_owned();
			if Path::new(&script).extension().is_some_and(|extension| extension == "py") {
				found.push(format!("{shown}:{at}: runs {script}, a python script"));
			}
			named.push(script);
		}
	}
}

/// What home runs: the Makefile and every module under nix/modules but the
/// modules' own test.
fn home_files(repo: &Path) -> Vec<PathBuf> {
	let mut files = vec![repo.join("Makefile")];
	let mut dirs = vec![repo.join("nix/modules")];
	while let Some(dir) = dirs.pop() {
		for entry in std::fs::read_dir(&dir).unwrap() {
			let path = entry.unwrap().path();
			if path.is_dir() {
				dirs.push(path);
			} else if path.extension().is_some_and(|extension| extension == "nix") && path.file_name().is_some_and(|name| name != "test.nix") {
				files.push(path);
			}
		}
	}
	files.sort();
	files
}

/// Every finding in `files` and, followed once each, the scripts they name.
fn scan(repo: &Path, files: Vec<PathBuf>) -> Vec<String> {
	let mut found = Vec::new();
	let mut queue: Vec<String> = Vec::new();
	for path in files {
		let shown = path.strip_prefix(repo).unwrap_or(&path).display().to_string();
		let text = std::fs::read_to_string(&path).unwrap();
		findings(&shown, &text, &mut found, &mut queue);
	}
	let mut seen: Vec<String> = Vec::new();
	while let Some(script) = queue.pop() {
		if seen.contains(&script) {
			continue;
		}
		seen.push(script.clone());
		if let Ok(text) = std::fs::read_to_string(repo.join(&script)) {
			findings(&script, &text, &mut found, &mut queue);
		}
	}
	found
}

fn repo() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

#[test]
fn nothing_home_runs_calls_python() {
	let repo = repo();
	let found = scan(&repo, home_files(&repo));
	assert!(
		found.is_empty(),
		"home has no python3, and these would run it there (write it in Rust, as envctl):\n{}",
		found.join("\n")
	);
}

#[test]
fn the_check_finds_what_failed_on_home() {
	let dir = std::env::temp_dir().join(format!("envctl-no-python-{}", std::process::id()));
	std::fs::create_dir_all(dir.join("scripts")).unwrap();
	// The recipe that failed on home, a module putting python on a unit's
	// PATH, and a shell script a recipe calls that runs python in turn.
	std::fs::write(
		dir.join("Makefile"),
		"# python3 in a comment is fine\nhome-deploy:\n\t@python3 scripts/deploy.py\nbuild:\n\t@scripts/build.sh\n",
	)
	.unwrap();
	std::fs::write(dir.join("unit.nix"), "{pkgs, ...}: {\n  # pkgs.python3 in a comment is fine\n  path = [pkgs.python3];\n}\n").unwrap();
	std::fs::write(dir.join("scripts/build.sh"), "#!/bin/sh\npython -c 'print(1)'\n").unwrap();
	std::fs::write(dir.join("scripts/hidden"), "#!/usr/bin/env python3\nprint(1)\n").unwrap();
	std::fs::write(dir.join("clean.nix"), "{pkgs, ...}: {\n  path = [pkgs.docker];\n  ExecStart = \"envctl deploy\";\n}\n").unwrap();
	let found = scan(&dir, vec![dir.join("Makefile"), dir.join("unit.nix")]);
	let has = |text: &str| found.iter().any(|line| line.contains(text));
	assert!(has("Makefile:3: @python3 scripts/deploy.py"), "{found:?}");
	assert!(has("Makefile:3: runs scripts/deploy.py, a python script"), "{found:?}");
	assert!(has("unit.nix:3: path = [pkgs.python3];"), "{found:?}");
	assert!(has("scripts/build.sh:2: python -c"), "{found:?}");
	assert!(!found.iter().any(|line| line.contains("comment")), "comments are not run: {found:?}");
	assert_eq!(found.len(), 4, "{found:?}");
	// A script found only by its shebang.
	std::fs::write(dir.join("Makefile"), "x:\n\t@scripts/hidden\n").unwrap();
	assert_eq!(scan(&dir, vec![dir.join("Makefile")]), ["scripts/hidden:1: its interpreter is python"]);
	assert!(scan(&dir, vec![dir.join("clean.nix")]).is_empty());
	std::fs::remove_dir_all(&dir).unwrap();
}
