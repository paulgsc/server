use serde::{Deserialize, Serialize};

// ── CacheEntry ────────────────────────────────────────────────────────────────
//
// Intentionally minimal. No timestamps, no access counters.
//
// Rationale:
//
//   - Age is derived from Redis TTL at read time: age = original_ttl - ttl_remaining.
//     Redis is the authoritative clock; storing `created_at` with SystemTime
//     introduces drift, NTP dependency, and a redundant write on every touch().
//
//   - Access counts belong in Prometheus counters, not stored payloads.
//     Writing them back to Redis on every get() turns every read into a write.
//     redis_exporter covers infrastructure hit/miss; application-level counters
//     (per-namespace, dedup contention) live in metrics.rs.
//
//   - `content_type` is absent; binary entries use BinaryCacheEntry.
//
// The only field alongside `data` is `ttl` (original, as set at write time),
// kept so age can be computed without an extra round-trip when TTL is already
// pipelined with GET.

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheEntry<T> {
	pub data: T,
	/// Original TTL in seconds. Age = original_ttl − ttl_remaining (Redis TTL command).
	pub ttl: u64,
}

impl<T> CacheEntry<T> {
	pub fn new(data: T, ttl: u64) -> Self {
		Self { data, ttl }
	}
}

// ── BinaryCacheEntry ──────────────────────────────────────────────────────────

/// Binary payload envelope — carries content-type across the Redis round-trip.
/// Same clock philosophy: no stored timestamps, age derived from Redis TTL.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BinaryCacheEntry {
	pub data: Vec<u8>,
	pub content_type: Option<String>,
	pub ttl: u64,
}

impl BinaryCacheEntry {
	pub fn new(data: Vec<u8>, content_type: Option<String>, ttl: u64) -> Self {
		Self { data, content_type, ttl }
	}
}
