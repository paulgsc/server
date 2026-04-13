//! TOML configuration schema.
//!
//! This is the user-facing surface for defining tracks and resources.
//! It is intentionally more ergonomic than the domain model — IDs are
//! derived from labels, parents are expressed as label strings, etc.
//!
//! A `TopologyBuilder` translates the raw config into validated domain
//! types, assigning numeric IDs and enforcing all structural invariants.
//!
//! # Example topology.toml
//!
//! ```toml
//! window_size = 20
//!
//! [[track]]
//! label    = "DSA"
//! target   = 10
//!
//! [[track]]
//! label    = "Arrays"
//! parent   = "DSA"
//! target   = 4
//! resources = ["arrays-neetcode", "leetcode-75-arrays"]
//!
//! [[track]]
//! label    = "Trees"
//! parent   = "DSA"
//! target   = 4
//! resources = ["bst-visualizer", "leetcode-75-trees", "sedgewick-ch3"]
//!
//! [[track]]
//! label    = "DP"
//! parent   = "DSA"
//! target   = 2
//! resources = ["dp-patterns-tab", "leetcode-75-dp"]
//! ```

use serde::Deserialize;

/// Root of the TOML config file.
#[derive(Debug, Deserialize)]
pub struct Config {
	/// Rolling window size (number of slots). Fairness is enforced over
	/// this window. Typical value: 10–30.
	pub window_size: usize,

	/// All tracks, in definition order. Parents must appear before children
	/// (enforced during build).
	#[serde(rename = "track")]
	pub tracks: Vec<TrackConfig>,
}

/// One `[[track]]` entry.
#[derive(Debug, Deserialize)]
pub struct TrackConfig {
	/// Unique human label. Used as the stable identifier in the config;
	/// numeric IDs are assigned during build.
	pub label: String,

	/// Label of the parent track. Omit for the root. There must be
	/// exactly one root across the whole config.
	pub parent: Option<String>,

	/// Sessions per window. Must be > 0.
	pub target: u32,

	/// Resource labels. Omit for internal (routing) tracks; required for
	/// leaf tracks. Order defines the cycle.
	#[serde(default)]
	pub resources: Vec<String>,
}

impl Config {
	pub fn from_toml(src: &str) -> Result<Self, toml::de::Error> {
		toml::from_str(src)
	}

	pub fn from_file(path: &std::path::Path) -> anyhow::Result<Self> {
		let src = std::fs::read_to_string(path)?;
		Ok(Self::from_toml(&src)?)
	}

	pub async fn from_cache(handle: &crate::cache::CacheHandle, key: &str) -> anyhow::Result<Option<Self>> {
		handle.get_topology(key).await
	}
}
