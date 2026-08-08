//
// Only application-layer signals that redis_exporter cannot provide.
//
// What redis_exporter already covers (do not duplicate here):
//   - Global hit/miss rate          → keyspace_hits / keyspace_misses
//   - Memory usage, eviction        → used_memory, evicted_keys
//   - Connection count, latency     → connected_clients, latency_histogram
//   - Command throughput            → instantaneous_ops_per_sec
//   - Per-keyspace key counts       → db{N}_keys
//
// What lives here (application-layer only), recorded through the `metrics`
// facade per #139 (P1) — this crate does not own a registry or an
// exposition path, only the measurements. The process embedding it decides
// where they go by installing a recorder (see `some-metrics`).
//
//   1. cache_hits_total / cache_misses_total — per logical namespace (key
//      prefix), recorded in `store.rs`. redis_exporter is global; we need
//      per-prefix breakdowns because `capture:session:*`, `embed:*`, etc.
//      have different SLOs.
//
//   2. cache_fetch_duration_seconds — upstream fetcher latency (the
//      DedupCache closure), recorded in `dedup.rs`. Redis has no visibility
//      into time spent fetching from upstream.
//
//   3. cache_dedup_waiters_total — count of requests that coalesced behind
//      an in-flight fetch (the `from_dedup` path in DedupCache), recorded in
//      `dedup.rs`. Tells you whether the thundering-herd guard is actually
//      firing and how much upstream pressure it absorbs. redis_exporter
//      cannot see this.

/// Extract a logical namespace from an unprefixed cache key.
/// "capture:session:abc123" → "capture:session"
/// "embed:xyz"              → "embed"
/// "simplekey"              → "simplekey"
#[must_use]
pub fn namespace_of(key: &str) -> &str {
	// Find the last ':' that precedes a non-structural segment (the id portion).
	// Convention: keys are `namespace:id` or `namespace:sub:id`.
	// We want everything up to but not including the final ':'-separated segment
	// if that segment looks like an id (contains no further ':').
	match key.rfind(':') {
		Some(pos) => &key[..pos],
		None => key,
	}
}

#[cfg(test)]
mod tests {
	use super::namespace_of;

	// #228 (C3): "confirm `namespace_of` produces a bounded label set for
	// the key patterns actually in use; a key with no `:` becomes its own
	// namespace." The `cache_hits_total{namespace}` label's cardinality is
	// bounded by the number of *prefixes* callers use, not by the number of
	// keys — these are the two ways that could fail to hold.

	#[test]
	fn collapses_varying_ids_under_the_same_prefix() {
		// A namespace with a thousand different ids is still one label
		// value, not a thousand — the whole point of prefix-based namespacing.
		for id in ["abc123", "def456", "0"] {
			assert_eq!(namespace_of(&format!("capture:session:{id}")), "capture:session");
		}
	}

	#[test]
	fn single_segment_key_is_its_own_namespace() {
		// The documented edge case: a key with no ':' at all has no prefix
		// to collapse onto, so it becomes its own (unbounded-in-principle)
		// namespace. Callers are responsible for not minting keys shaped
		// this way at unbounded cardinality — `namespace_of` cannot enforce
		// a convention its caller didn't follow, only report it faithfully.
		assert_eq!(namespace_of("simplekey"), "simplekey");
	}

	#[test]
	fn two_segment_key_is_the_prefix_alone() {
		assert_eq!(namespace_of("embed:xyz"), "embed");
	}
}
