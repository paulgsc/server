/// Command handlers.
///
/// Each handler is responsible for exactly one subcommand.
/// They orchestrate: store → model → query → store,
/// but contain no parsing logic and no direct file I/O.
///
/// Separation contract:
///   - `store::*`  owns file I/O
///   - `model::*`  owns domain invariants
///   - `query::*`  owns fuzzy matching and picker UX
///   - handlers    own orchestration and user-facing output
use std::io::{self, Read, Write};

use crate::{
	error::{PocketError, Result},
	model::{Pending, Register},
	query, store,
	store::RegistryPath,
};

// ── add ───────────────────────────────────────────────────────────────────────

/// Resolve the label from the argument or by prompting the user.
fn resolve_label(label: Option<String>) -> Result<String> {
	match label {
		Some(l) => Ok(l),
		None => {
			eprint!("Label: ");
			io::stderr().flush().ok();
			let mut buf = String::new();
			io::stdin().read_line(&mut buf).map_err(|source| PocketError::StdinRead { source })?;
			Ok(buf.trim().to_string())
		}
	}
}

/// Resolve the value from the argument or by reading all of stdin.
fn resolve_value(value: Option<String>) -> Result<String> {
	match value {
		Some(v) => Ok(v),
		None => {
			let mut buf = String::new();
			io::stdin().read_to_string(&mut buf).map_err(|source| PocketError::StdinRead { source })?;
			Ok(buf)
		}
	}
}

pub fn add(path: &RegistryPath, label: Option<String>, value: Option<String>) -> Result<()> {
	let label = resolve_label(label)?;
	let value = resolve_value(value)?;

	// Build and validate the register before touching the registry.
	let reg = Register::<Pending>::from_input(label, value).validate()?;

	let mut registry = store::load_or_init(path)?;
	registry.add(reg)?;
	store::save(&registry, path)?;

	eprintln!("saved.");
	Ok(())
}

// ── query ─────────────────────────────────────────────────────────────────────

pub fn query(path: &RegistryPath) -> Result<()> {
	let registry = store::load_or_init(path)?;

	if registry.is_empty() {
		eprintln!("registry is empty. Use `pocket add` to create entries.");
		return Ok(());
	}

	let registers: Vec<_> = registry.iter().map(|r| r.into_data()).collect();
	let value = query::pick_interactive(&registers)?;

	// Emit the value to stdout so the caller can pipe it anywhere.
	print!("{}", value);
	io::stdout().flush().ok();
	Ok(())
}

// ── ls ────────────────────────────────────────────────────────────────────────

pub fn ls(path: &RegistryPath) -> Result<()> {
	let registry = store::load_or_init(path)?;

	if registry.is_empty() {
		eprintln!("(no registers)");
		return Ok(());
	}

	let mut labels: Vec<String> = registry.iter().map(|r| r.as_data().label.clone()).collect();
	labels.sort();
	for label in labels {
		println!("{}", label);
	}
	Ok(())
}

// ── rm ────────────────────────────────────────────────────────────────────────

pub fn rm(path: &RegistryPath, label: &str) -> Result<()> {
	let mut registry = store::load_or_init(path)?;
	let removed = registry.remove(label)?;
	store::save(&registry, path)?;
	eprintln!("removed: {}", removed.label);
	Ok(())
}

// ── edit ─────────────────────────────────────────────────────────────────────

pub fn edit(path: &RegistryPath, label: &str) -> Result<()> {
	let mut registry = store::load_or_init(path)?;

	let reg = registry.find(label).ok_or_else(|| PocketError::LabelNotFound { label: label.to_string() })?;

	// Write value to a temp file, open $EDITOR, read back.
	let mut tmp = tempfile::NamedTempFile::new().map_err(|source| PocketError::RegistryWrite {
		path: std::path::PathBuf::from("<tempfile>"),
		source,
	})?;

	{
		use std::io::Write;
		tmp.write_all(reg.as_data().value.as_bytes()).map_err(|source| PocketError::RegistryWrite {
			path: tmp.path().to_path_buf(),
			source,
		})?;
	}

	let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
	let status = std::process::Command::new(&editor).arg(tmp.path()).status().map_err(|source| PocketError::RegistryWrite {
		path: tmp.path().to_path_buf(),
		source,
	})?;

	if !status.success() {
		eprintln!("editor exited with non-zero status; no changes saved.");
		return Ok(());
	}

	let new_value = std::fs::read_to_string(tmp.path()).map_err(|source| PocketError::RegistryRead {
		path: tmp.path().to_path_buf(),
		source,
	})?;

	// Remove old, insert updated.  Validate first so an empty edit is rejected.
	let updated = Register::<Pending>::from_input(label, new_value).validate()?;
	registry.remove(label)?;
	registry.add(updated)?;
	store::save(&registry, path)?;

	eprintln!("updated: {}", label);
	Ok(())
}

// ── path ─────────────────────────────────────────────────────────────────────

pub fn print_path(path: &RegistryPath) {
	println!("{}", path);
}
