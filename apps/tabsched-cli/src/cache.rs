//! Redis cache wiring for the `ts` CLI.
//!
//! Concerns:
//! * Constructing a `CacheStore` from env / defaults.
//! * `pull_updates`: reads the `pipeline:completed` stream, fetches each
//!   session's TOML artifact from Redis, and writes it to `topology.toml`
//!   on the local filesystem.
//!
//! **Only `ts init` and `ts pull` touch Redis.**  All other commands read
//! `topology.toml` directly from the filesystem via `Ctx::load`.

use std::path::Path;

use anyhow::{Context, Result};
use some_cache::{CacheConfig, CacheStore, StreamHandle};
use tracing::{info, instrument, warn};

/// Key prefix used for all keys written by this bin.
const KEY_PREFIX: &str = "";

// ── CacheHandle ───────────────────────────────────────────────────────────────

/// Thin wrapper around `CacheStore`.  Only used during `init` / `pull`.
#[derive(Clone)]
pub struct CacheHandle {
	store: CacheStore,
}

impl CacheHandle {
	/// Construct from `REDIS_URL` env var (falls back to `redis://127.0.0.1:6379`).
	pub fn from_env() -> anyhow::Result<Self> {
		let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

		let config = CacheConfig::new(redis_url).with_prefix(KEY_PREFIX).with_compression(true, 512);

		let store = CacheStore::new(config).context("constructing CacheStore")?;
		Ok(Self { store })
	}

	/// Drain pending and new entries from the `pipeline:completed` stream,
	/// writing the latest topology TOML to `<data_dir>/topology.toml`.
	///
	/// Returns the number of entries that produced a successful file write.
	/// The file is rewritten on each applied entry; the last one wins, which
	/// is correct because entries are ordered and we want the newest artifact.
	#[instrument(skip(self, data_dir), fields(dir = %data_dir.display()))]
	pub async fn pull_updates(&self, data_dir: &Path, consumer_id: &str) -> Result<usize> {
		let stream = StreamHandle::pipeline_completed(self.store.clone(), consumer_id);

		info!(consumer_id, "checking for topology updates");

		stream.ensure_group(false).await.context("ensure consumer group")?;

		let mut total = 0;

		// 1. Recover pending (PEL) — entries we received but never ACKed.
		let pending = stream.read_pending(32).await?;
		if !pending.is_empty() {
			info!(count = pending.len(), "recovering unacknowledged updates");
			total += self.apply_stream_entries(&stream, data_dir, &pending).await?;
		}

		// 2. New entries.
		let new = stream.read_new(64, 2000).await?;
		if !new.is_empty() {
			info!(count = new.len(), "applying new updates");
			total += self.apply_stream_entries(&stream, data_dir, &new).await?;
		} else {
			info!("no new updates");
		}

		if total > 0 {
			info!(total, "sync complete");
		}

		Ok(total)
	}

	#[instrument(skip(self, stream, data_dir, entries), fields(batch_size = entries.len()))]
	async fn apply_stream_entries(&self, stream: &StreamHandle, data_dir: &Path, entries: &[(String, String)]) -> Result<usize> {
		let mut count = 0;

		for (entry_id, session_id) in entries {
			match self.fetch_and_write_toml(data_dir, session_id).await {
				Ok(true) => {
					info!(session_id, "applied update");
					count += 1;
				}
				Ok(false) => {
					warn!(session_id, "artifact missing in redis (likely expired)");
				}
				Err(e) => {
					warn!(session_id, error = %e, "failed to fetch/write update; skipping");
				}
			}

			// Always ACK — don't get stuck replaying a poison entry.
			if let Err(e) = stream.ack(&[entry_id.clone()]).await {
				warn!(entry_id, error = %e, "failed to acknowledge stream entry");
			}
		}

		Ok(count)
	}

	/// Fetch `pipeline:artifact:{session_id}:toml` from Redis and overwrite
	/// `<data_dir>/topology.toml` on the local filesystem.
	///
	/// Returns `Ok(false)` if the artifact key is absent (expired / not yet
	/// written).  Returns `Ok(true)` on a successful file write.
	#[instrument(skip(self, data_dir), fields(session_id = %session_id))]
	async fn fetch_and_write_toml(&self, data_dir: &Path, session_id: &str) -> Result<bool> {
		let art_key = format!("pipeline:artifact:{}:toml", session_id);

		let toml_str: Option<String> = self.store.get(&art_key).await.with_context(|| format!("fetching artifact {}", session_id))?;

		let Some(toml) = toml_str else {
			return Ok(false);
		};

		// Validate before touching the filesystem — don't write a corrupt file.
		crate::config::Config::from_toml(&toml).with_context(|| format!("artifact {} is not valid topology TOML", session_id))?;

		std::fs::create_dir_all(data_dir).context("creating data_dir")?;

		let toml_path = data_dir.join("topology.toml");
		std::fs::write(&toml_path, &toml).with_context(|| format!("writing {}", toml_path.display()))?;

		info!(path = %toml_path.display(), "topology.toml updated");
		Ok(true)
	}
}
