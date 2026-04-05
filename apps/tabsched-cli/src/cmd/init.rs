//! `ts init` — write a starter `topology.toml` to the data directory.
//!
//! Safe to run multiple times: will not overwrite an existing file.
//! Prints the path so the user knows where to edit.

use std::path::Path;

use anyhow::{bail, Result};
use owo_colors::OwoColorize;

const TEMPLATE: &str = r#"# tabsched topology
#
# window_size: rolling window over which fairness is enforced (slots).
# A value of 20 means "over the last 20 sessions, each track should
# receive approximately its target share."
window_size = 20

# ── Tracks ──────────────────────────────────────────────────────────────────
#
# Internal tracks (no resources) are routing nodes.
# Leaf tracks (with resources) are execution units.
# Every leaf must have a parent; there must be exactly one root
# (a track with no parent field).

[[track]]
label  = "DSA"
target = 10

[[track]]
label     = "Arrays"
parent    = "DSA"
target    = 4
resources = ["neetcode-arrays", "lc75-arrays"]

[[track]]
label     = "Trees"
parent    = "DSA"
target    = 4
resources = ["bst-visualizer", "lc75-trees", "sedgewick-ch3"]

[[track]]
label     = "DP"
parent    = "DSA"
target    = 2
resources = ["dp-patterns", "lc75-dp"]
"#;

pub fn run(data_dir: &Path) -> Result<()> {
	let path = data_dir.join("topology.toml");

	if path.exists() {
		bail!(
			"topology.toml already exists at {}\n  \
             Edit it directly, or delete it to re-run init.",
			path.display()
		);
	}

	std::fs::create_dir_all(data_dir)?;
	std::fs::write(&path, TEMPLATE)?;

	println!("\n  created {}\n  edit it, then run {}\n", path.display().bright_cyan(), "ts status".bright_cyan());
	Ok(())
}
