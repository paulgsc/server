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
// What lives here (application-layer only):
//
//   1. CACHE_HITS / CACHE_MISSES — per logical namespace (key prefix).
//      redis_exporter is global; we need per-prefix breakdowns because
//      `capture:session:*`, `embed:*`, etc. have different SLOs.
//
//   2. FETCH_DURATION — upstream fetcher latency (the DedupCache closure).
//      Redis has no visibility into time spent fetching from upstream.
//      This is pure application signal.
//
//   3. DEDUP_WAITERS — count of requests that coalesced behind an in-flight
//      fetch (the `from_dedup` path in DedupCache). Tells you whether the
//      thundering-herd guard is actually firing and how much upstream pressure
//      it absorbs. redis_exporter cannot see this.
//
// If none of these are being read in dashboards, delete this file and rely
// solely on redis_exporter + the official Grafana dashboard.

use once_cell::sync::Lazy;
use prometheus::{register_counter_vec, register_histogram_vec, CounterVec, HistogramVec};

// ── Per-namespace hit/miss ────────────────────────────────────────────────────
//
// Label: `namespace` — the key prefix, e.g. "capture:session", "embed".
// Derive it from the unprefixed key at the call site: split on ':' and take
// the first segment, or pass it explicitly.

pub static CACHE_HITS: Lazy<Result<CounterVec, prometheus::Error>> =
	Lazy::new(|| register_counter_vec!("cache_hits_total", "Cache hits by logical namespace", &["namespace"]));

pub static CACHE_MISSES: Lazy<Result<CounterVec, prometheus::Error>> =
	Lazy::new(|| register_counter_vec!("cache_misses_total", "Cache misses by logical namespace", &["namespace"]));

// ── Upstream fetch latency ────────────────────────────────────────────────────
//
// Label: `namespace` — same as above.
// Measured from the start of the fetcher closure to Redis write completion.
// Buckets: tuned for typical upstream fetch latency (ms to low seconds).

pub static FETCH_DURATION: Lazy<Result<HistogramVec, prometheus::Error>> = Lazy::new(|| {
	register_histogram_vec!(
		"cache_fetch_duration_seconds",
		"Upstream fetcher latency (DedupCache miss path) by namespace",
		&["namespace"],
		vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]
	)
});

// ── Dedup contention ─────────────────────────────────────────────────────────
//
// Label: `namespace`.
// Incremented when a request coalesces behind an in-flight fetch (waiter path).
// Rate of this counter / rate of CACHE_MISSES = dedup efficiency ratio.

pub static DEDUP_WAITERS: Lazy<Result<CounterVec, prometheus::Error>> = Lazy::new(|| {
	register_counter_vec!(
		"cache_dedup_waiters_total",
		"Requests that coalesced behind an in-flight fetch (thundering-herd guard)",
		&["namespace"]
	)
});

// ── Helper ────────────────────────────────────────────────────────────────────

/// Extract a logical namespace from an unprefixed cache key.
/// "capture:session:abc123" → "capture:session"
/// "embed:xyz"              → "embed"
/// "simplekey"              → "simplekey"
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
