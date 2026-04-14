//! `ts pull` — sync topology from the pipeline daemon via Redis Stream.
//!
//! Safe to run at any time, including before `topology.toml` exists.
//! Idempotent: replays any missed events, ACKs each entry exactly once.

use anyhow::Result;
use std::path::Path;

use crate::cache::CacheHandle;

pub async fn run(data_dir: &Path) -> Result<()> {
	let handle = CacheHandle::from_env()?;

	// Stable consumer name: use hostname so different machines maintain
	// independent positions in the consumer group.
	let consumer = hostname::get().map(|h| h.to_string_lossy().into_owned()).unwrap_or_else(|_| "ts-cli".to_string());

	let applied = handle.pull_updates(data_dir, &consumer).await?;

	if applied == 0 {
		println!("topology is up to date");
	} else {
		println!("applied {} topology update(s)", applied);
	}

	Ok(())
}
