//! Runtime context shared across all command handlers.
//!
//! `Ctx` is the single owner of mutable scheduler state within a CLI
//! invocation.  It is constructed once in `main`, then passed by `&mut`
//! into each command handler.  No global state, no lazy statics.
//!
//! `Ctx::load` reads only from the local filesystem — no network I/O.
//! Redis is touched exclusively by `ts init` and `ts pull` (see `cache.rs`).

use std::path::{Path, PathBuf};

use anyhow::Context;
use tabsched::{Engine, State, Topology};

use crate::config::LabelIndex;

pub struct Ctx {
	pub engine: Engine<'static>,
	pub topology: &'static Topology,
	pub index: LabelIndex,
	pub data_dir: PathBuf,
}

impl Ctx {
	/// Load scheduler state from `data_dir` — filesystem only.
	///
	/// Reads:
	/// - `topology.toml`  — track/resource definitions (required; written by
	///                       `ts pull` or manually by the user)
	/// - `state.json`     — persisted session history (created on first run)
	///
	/// The `Topology` is leaked to `'static` so `Engine` can hold a reference
	/// without lifetime propagation infecting every call-site.  Acceptable
	/// because there is exactly one `Ctx` per process, `Topology` is immutable
	/// after construction, and the process exits when the command completes.
	pub fn load(data_dir: &Path) -> anyhow::Result<Self> {
		let toml_path = data_dir.join("topology.toml");
		let state_path = data_dir.join("state.json");

		let config = crate::config::Config::from_file(&toml_path).with_context(|| format!("reading {} (run `ts pull` to fetch from pipeline)", toml_path.display()))?;

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
