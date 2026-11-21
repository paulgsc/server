//! No `reqwest::Client` is named outside this crate (paulgsc/some-ui#3330).
//!
//! `clippy.toml` refuses `reqwest::Client::new`, `reqwest::Client::builder` and
//! `reqwest::ClientBuilder::build` anywhere else, but a lint path cannot name
//! a trait method, so `reqwest::Client::default()` (or a `#[derive(Default)]`
//! around a field of that type) would slip past it. An alias
//! (`use reqwest as r;`) still would; none exists, and review is that guard. This test closes that: outside this crate,
//! nothing in `apps/` or `crates/` names the type at all, so nothing can hold
//! a client this crate did not build.

use std::path::{Path, PathBuf};

/// Every `.rs` file under `dir`, skipping build output.
fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
	let Ok(entries) = std::fs::read_dir(dir) else { return };
	for entry in entries.flatten() {
		let path = entry.path();
		let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
		if path.is_dir() {
			if name != "target" && !name.starts_with('.') && name != "node_modules" {
				sources(&path, out);
			}
		} else if path.extension().is_some_and(|ext| ext == "rs") {
			out.push(path);
		}
	}
}

/// Whether `line` names `reqwest::Client` (not `reqwest::ClientBuilder`).
fn names_the_client(line: &str) -> bool {
	line.match_indices("reqwest::Client").any(|(at, found)| {
		let next = line[at + found.len()..].chars().next();
		!next.is_some_and(|c| c.is_alphanumeric() || c == '_')
	})
}

/// Whether a `use reqwest::{...}` group in `text` imports `Client`, however
/// rustfmt split it across lines and however deeply it nests.
fn imports_the_client(text: &str) -> bool {
	text.match_indices("use reqwest::{").any(|(at, found)| {
		let group = &text[at + found.len()..];
		let mut depth = 1_usize;
		let end = group
			.char_indices()
			.find(|&(_, c)| {
				match c {
					'{' => depth += 1,
					'}' => depth -= 1,
					_ => {}
				}
				depth == 0
			})
			.map_or(group.len(), |(end, _)| end);
		group[..end].split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|word| word == "Client")
	})
}

#[test]
fn nothing_outside_this_crate_names_a_reqwest_client() {
	let here = Path::new(env!("CARGO_MANIFEST_DIR"));
	let root = here.parent().unwrap().parent().unwrap();
	let mut files = Vec::new();
	sources(&root.join("apps"), &mut files);
	sources(&root.join("crates"), &mut files);
	assert!(files.len() > 100, "found only {} sources under {}", files.len(), root.display());

	let mut offenders = Vec::new();
	for file in files.iter().filter(|file| !file.starts_with(here)) {
		let text = std::fs::read_to_string(file).unwrap();
		let path = file.strip_prefix(root).unwrap().display().to_string();
		for (number, line) in text.lines().enumerate() {
			if names_the_client(line) {
				offenders.push(path.clone() + ":" + &(number + 1).to_string());
			}
		}
		if imports_the_client(&text) {
			offenders.push(path + ": use reqwest::{.., Client, ..}");
		}
	}
	assert!(
		offenders.is_empty(),
		"build the client with outbound::client(Caller::...), not reqwest directly: {offenders:?}"
	);
}

#[test]
fn the_check_sees_each_way_to_name_the_client() {
	assert!(names_the_client("let c = reqwest::Client::default();"));
	assert!(names_the_client("	client: reqwest::Client,"));
	assert!(imports_the_client("use reqwest::{header, Client};"));
	assert!(imports_the_client("use reqwest::{\n\theader::{self, HeaderName},\n\tClient,\n};"));
	assert!(!imports_the_client("use reqwest::{header::{self}, Url};\nstruct Client;"));
	assert!(!names_the_client("let b = reqwest::ClientBuilder::new();"));
	assert!(!names_the_client("let c: outbound::Client = todo!();"));
}
