// cache-panels.libsonnet
//
// #212/#217/#220: rebuilt from scratch against the 4 metric families
// `some-cache` actually records (see crates/some-cache/src/metrics.rs's
// header comment — redis_exporter already covers global hit/miss rate,
// memory, connections and throughput; this file only queries what's
// application-layer). The previous ~20-panel version queried
// `cache_operations_total`, `cache_compression_ratio`, `cache_ttl_seconds`
// and a dozen other names nothing in this workspace has ever emitted.
//
// Query strings are inlined rather than routed through a shared constants
// table — scripts/check_metric_contract.py (#217) reads `expr:` literally
// out of this file, the same way every other panel library in this repo
// does, so an indirection here would just be a blind spot for that check.
local utils = import 'cache-utils.libsonnet';

{
  // === Overview stats ===

  cacheHitRate: utils.createStatPanel(
    'Cache Hit Rate',
    'sum(rate(cache_hits_total[5m])) / (sum(rate(cache_hits_total[5m])) + sum(rate(cache_misses_total[5m]))) * 100',
    utils.units.percent,
    utils.thresholds.hitRate.steps,
  ) { fieldConfig+: { defaults+: { min: 0, max: 100 } } },

  cacheOpsRate: utils.createStatPanel(
    'Cache Ops/sec (hits + misses)',
    'sum(rate(cache_hits_total[5m])) + sum(rate(cache_misses_total[5m]))',
    utils.units.ops,
    utils.thresholds.operations.low.steps,
  ),

  dedupWaitersRate: utils.createStatPanel(
    // Tells you whether the thundering-herd guard in `DedupCache` is
    // actually firing — a request that coalesces behind an in-flight fetch
    // shows up here, and redis_exporter has no visibility into it at all.
    'Dedup Waiters/sec',
    'sum(rate(cache_dedup_waiters_total[5m]))',
    utils.units.ops,
    utils.thresholds.operations.low.steps,
  ),

  // === Per-namespace detail ===

  cacheHitsAndMissesByNamespace: utils.createTimeseriesPanel(
    'Hits vs Misses by Namespace',
    [
      { expr: 'sum(rate(cache_hits_total[5m])) by (namespace)', legendFormat: '{{namespace}} - hit' },
      { expr: 'sum(rate(cache_misses_total[5m])) by (namespace)', legendFormat: '{{namespace}} - miss' },
    ],
    utils.units.ops,
  ),

  dedupWaitersByNamespace: utils.createTimeseriesPanel(
    'Dedup Waiters by Namespace',
    [{ expr: 'sum(rate(cache_dedup_waiters_total[5m])) by (namespace)', legendFormat: '{{namespace}}' }],
    utils.units.ops,
  ),

  cacheFetchDuration: utils.createTimeseriesPanel(
    'Upstream Fetch Duration (P50/P95/P99)',
    [
      { expr: 'histogram_quantile(0.50, sum(rate(cache_fetch_duration_seconds_bucket[5m])) by (le, namespace))', legendFormat: '{{namespace}} - p50' },
      { expr: 'histogram_quantile(0.95, sum(rate(cache_fetch_duration_seconds_bucket[5m])) by (le, namespace))', legendFormat: '{{namespace}} - p95' },
      { expr: 'histogram_quantile(0.99, sum(rate(cache_fetch_duration_seconds_bucket[5m])) by (le, namespace))', legendFormat: '{{namespace}} - p99' },
    ],
    utils.units.seconds,
  ),
}
