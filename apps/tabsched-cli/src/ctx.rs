//! Runtime context shared across all command handlers.
//!
//! `Ctx` is the single owner of mutable scheduler state within a CLI
//! invocation. It is constructed once in `main`, then passed by `&mut`
//! into each command handler. No global state, no lazy statics.
//!
//! `Ctx` also knows its own data directory so command handlers do not
//! need to construct paths themselves.

use std::path::{Path, PathBuf};

use crate::cache::CacheHandle;
use anyhow::Context;
use tabsched::{Engine, State, Topology};
use tracing::warn;

use crate::config::LabelIndex;

pub struct Ctx {
	pub engine: Engine<'static>,
	pub topology: &'static Topology,
	pub index: LabelIndex,
	pub data_dir: PathBuf,
}

impl Ctx {
	/// Load or initialise scheduler state from `data_dir`.
	///
	/// - `topology.toml`  — track/resource definitions (required)
	/// - `state.json`     — persisted session history (created on first run)
	///
	/// The `Topology` is leaked to `'static` so `Engine` can hold a
	/// reference without lifetime propagation infecting every call-site.
	/// This is acceptable because:
	/// 1. There is exactly one `Ctx` per process.
	/// 2. `Topology` is immutable after construction.
	/// 3. The process exits when the CLI command completes.
	/// Async because topology may be fetched from Redis.
	pub async fn load(data_dir: &Path) -> anyhow::Result<Self> {
		let toml_path = data_dir.join("topology.toml");
		let state_path = data_dir.join("state.json");

		// ── Build cache handle (non-fatal on failure)
		let cache = CacheHandle::from_env().unwrap_or_else(|e| {
			warn!("cache unavailable, falling back to FS only: {}", e);
			// Construct a no-op handle that will always miss.
			// from_env() only fails on bad redis URL construction, not
			// on connection - so this path is unusual.
			CacheHandle::from_env().expect("infallible default redis URL")
		});

		let cache_key = CacheHandle::topology_key(data_dir);
		let config = match crate::config::Config::from_cache(&cache, &cache_key).await {
			Ok(Some(cached)) => {
				tracing::debug!("topology cache hit ({})", cache_key);
				cached
			}
			Ok(None) => {
				tracing::debug!("topology cache miss, reading from FS");
				let cfg = crate::config::Config::from_file(&toml_path).with_context(|| format!("reading {}", toml_path.display()))?;
				cfg
			}
			Err(e) => {
				warn!("topology cache get failed, falling back to FS: {}", e);
				crate::config::Config::from_file(&toml_path).with_context(|| format!("reading {}", toml_path.display()))?
			}
		};

		let (topology, index) = crate::config::build(&config).context("building topology from config")?;

		// Leak to 'static.
		let topology: &'static Topology = Box::leak(Box::new(topology));

		let state = if state_path.exists() {
			tabsched::adapters::snapshot::load(&state_path, topology).with_context(|| format!("loading state from {}", state_path.display()))?
		} else {
			State::new(topology, config.window_size)
		};

		let engine = Engine::new(state, topology);

		Ok(Self {
			engine,
			topology,
			index,
			data_dir: data_dir.to_owned(),
		})
	}

	/// Persist current state to `state.json`.
	pub fn save(&self) -> anyhow::Result<()> {
		let path = self.data_dir.join("state.json");
		tabsched::adapters::snapshot::save(self.engine.state(), &path).with_context(|| format!("saving state to {}", path.display()))?;
		Ok(())
	}
}
