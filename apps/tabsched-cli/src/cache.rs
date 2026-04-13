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

use anyhow::Context;
use some_cache::{CacheConfig, CacheStore};

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
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// FNV-1a 32-bit hash.  No dep needed; only used for key scoping.
fn fnv1a_32(data: &[u8]) -> u32 {
	const OFFSET: u32 = 2166136261;
	const PRIME: u32 = 16777619;
	data.iter().fold(OFFSET, |h, &b| (h ^ b as u32).wrapping_mul(PRIME))
}
