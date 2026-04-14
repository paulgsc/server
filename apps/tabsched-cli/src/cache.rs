//! Redis cache wiring for the `ts` CLI.
//!
//! Thin adapter between `some-cache` and this bin.  The only concerns here
//! are:
//!
//! * Constructing a `CacheStore` from env / defaults.
//! * The `topology:` key namespace and TTL policy.
//! * A `From<CacheConfig>` shim (kept here to avoid a circular dep on
//!   `some-cache` knowing about this bin's config shape).
//!
//! Callers should use [`CacheHandle`] rather than reaching for `CacheStore`
//! directly; it bakes in the key prefix and TTL so call-sites stay clean.

use std::path::Path;

use anyhow::{Context, Result};
use some_cache::{CacheConfig, CacheStore, StreamHandle};
use tracing::{info, instrument, warn};

/// Default TTL for cached topology configs (24 h).
///
/// Long because topology is user-edited, not written by any daemon.
/// Override with `TS_TOPOLOGY_TTL` (seconds).
const DEFAULT_TOPOLOGY_TTL: u64 = 86_400;

/// Key prefix used for all keys written by this bin.
const KEY_PREFIX: &str = "ts:";

// ── CacheHandle ───────────────────────────────────────────────────────────────

/// Thin wrapper around `CacheStore` that enforces this bin's key conventions.
#[derive(Clone)]
pub struct CacheHandle {
	store: CacheStore,
}

impl CacheHandle {
	/// Construct from `REDIS_URL` env var (falls back to `redis://127.0.0.1:6379`).
	pub fn from_env() -> anyhow::Result<Self> {
		let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
		let topology_ttl = std::env::var("TS_TOPOLOGY_TTL").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(DEFAULT_TOPOLOGY_TTL);

		let config = CacheConfig::new(redis_url).with_ttl(topology_ttl).with_prefix(KEY_PREFIX).with_compression(true, 512);

		let store = CacheStore::new(config).context("constructing CacheStore")?;
		Ok(Self { store })
	}

	/// Canonical cache key for a topology rooted at `data_dir`.
	///
	/// Scoped by directory so multiple `ts` invocations against different
	/// data dirs don't collide.  The key is stable across runs for the same
	/// directory.
	///
	/// Format: `topology:{hex8}` where hex8 is the lower 32 bits of a
	/// FNV-1a hash of the canonical path string.  Collision probability is
	/// negligible for the expected cardinality (< 100 directories per host).
	pub fn topology_key(data_dir: &Path) -> String {
		let canonical = data_dir.to_string_lossy();
		let hash = fnv1a_32(canonical.as_bytes());
		format!("topology:{:08x}", hash)
	}

	/// Fetch a cached topology config.  Returns `None` on miss.
	pub async fn get_topology(&self, key: &str) -> anyhow::Result<Option<crate::config::Config>> {
		self.store.get::<crate::config::Config>(key).await.context("cache get topology")
	}

	#[instrument(skip(self, data_dir), fields(dir = %data_dir.display()))]
	pub async fn pull_updates(&self, data_dir: &Path, consumer_id: &str) -> Result<usize> {
		let stream = StreamHandle::pipeline_completed(self.store.clone(), consumer_id);

		info!(consumer_id, "checking for topology updates");

		stream.ensure_group(false).await.context("ensure consumer group")?;

		let mut total = 0;

		// 1. Recover Pending (PEL)
		let pending = stream.read_pending(32).await?;
		if !pending.is_empty() {
			info!(count = pending.len(), "recovering unacknowledged updates");
			total += self.apply_stream_entries(&stream, data_dir, &pending).await?;
		}

		// 2. New Entries
		// We use a shorter block for CLI feel
		let new = stream.read_new(64, 2000).await?;
		if !new.is_empty() {
			info!(count = new.len(), "applying new updates");
			total += self.apply_stream_entries(&stream, data_dir, &new).await?;
		} else {
			info!("cache is up to date");
		}

		if total > 0 {
			info!(total, "sync complete");
		}

		Ok(total)
	}

	#[instrument(skip(self, stream, data_dir, entries), fields(batch_size = entries.len()))]
	async fn apply_stream_entries(&self, stream: &StreamHandle, data_dir: &Path, entries: &[(String, String)]) -> Result<usize> {
		let cache_key = Self::topology_key(data_dir);
		let mut count = 0;

		for (entry_id, session_id) in entries {
			match self.sync_session_to_topology(&cache_key, session_id).await {
				Ok(true) => {
					info!(session_id, "applied update");
					count += 1;
				}
				Ok(false) => {
					warn!(session_id, "artifact missing in redis (likely expired)");
				}
				Err(e) => {
					warn!(session_id, error = %e, "failed to parse/apply update; skipping");
				}
			}

			// Always ACK so we don't get stuck in a PEL loop with a "poison" entry
			if let Err(e) = stream.ack(&[entry_id.clone()]).await {
				warn!(entry_id, error = %e, "failed to acknowledge stream entry");
			}
		}
		Ok(count)
	}

	#[instrument(skip(self, cache_key), fields(session_id = %session_id))]
	async fn sync_session_to_topology(&self, cache_key: &str, session_id: &str) -> Result<bool> {
		let art_key = format!("pipeline:artifact:{}:toml", session_id);

		let toml_str: Option<String> = self.store.get(&art_key).await.with_context(|| format!("fetching artifact {}", session_id))?;

		let Some(s) = toml_str else { return Ok(false) };

		let config = crate::config::Config::from_toml(&s).with_context(|| "failed to parse pipeline TOML")?;

		self.store.set(cache_key, &config, None).await.with_context(|| "failed to write to local topology cache")?;

		Ok(true)
	}
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// FNV-1a 32-bit hash.  No dep needed; only used for key scoping.
fn fnv1a_32(data: &[u8]) -> u32 {
	const OFFSET: u32 = 2166136261;
	const PRIME: u32 = 16777619;
	data.iter().fold(OFFSET, |h, &b| (h ^ b as u32).wrapping_mul(PRIME))
}
