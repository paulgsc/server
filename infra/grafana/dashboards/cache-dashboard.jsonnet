// cache-dashboard.jsonnet
//
// #212/#217/#220: rebuilt against the 4 metric families `some-cache`
// actually records — `cache_hits_total` / `cache_misses_total` /
// `cache_fetch_duration_seconds` / `cache_dedup_waiters_total`, all labeled
// by `namespace`. The previous version queried 14 other names (
// `cache_operations_total`, `cache_compression_ratio`, `cache_ttl_seconds`,
// …) that no crate in this workspace has ever emitted; #139 already
// settled that only these 4 survive (see crates/some-cache/src/metrics.rs).
local panels = import 'lib/cache/cache-panels.libsonnet';
local utils = import 'lib/cache/cache-utils.libsonnet';
local panelDefaults = import 'lib/panel-defaults.libsonnet';

{
  annotations: {
    list: [
      {
        builtIn: 1,
        // Grafana's own built-in "Annotations & Alerts" store, not
        // Prometheus — always present under this literal uid. See #218.
        datasource: { type: 'grafana', uid: '-- Grafana --' },
        enable: true,
        hide: true,
        iconColor: 'rgba(0, 211, 255, 1)',
        name: 'Annotations & Alerts',
        type: 'dashboard',
      },
    ],
  },
  description: 'some-cache: the application-layer cache signals redis_exporter cannot see.',
  editable: true,
  fiscalYearStartMonth: 0,
  graphTooltip: 1,
  id: null,
  links: [],
  liveNow: true,
  panels: panelDefaults.hardenAll([
    utils.row('Cache Overview', 0, 0) { id: 1 },

    panels.cacheHitRate { gridPos: utils.gridPos(0, 1, 6, 4), id: 2, datasource: utils.datasource },
    panels.cacheOpsRate { gridPos: utils.gridPos(6, 1, 6, 4), id: 3, datasource: utils.datasource },
    panels.dedupWaitersRate { gridPos: utils.gridPos(12, 1, 6, 4), id: 4, datasource: utils.datasource },
    // #219: if this goes grey, `file_host` isn't being scraped — the other
    // three stats on this row would otherwise read as "cache is idle".
    panelDefaults.livenessPanel('file_host liveness', ['file_host'], 5, utils.gridPos(18, 1, 6, 4)),

    utils.row('Hit/Miss by Namespace', 0, 5) { id: 6 },
    panels.cacheHitsAndMissesByNamespace { gridPos: utils.gridPos(0, 6, 24, 8), id: 7, datasource: utils.datasource },

    utils.row('Dedup (Thundering-Herd Guard)', 0, 14) { id: 8 },
    panels.dedupWaitersByNamespace { gridPos: utils.gridPos(0, 15, 24, 8), id: 9, datasource: utils.datasource },

    utils.row('Upstream Fetch Latency', 0, 23) { id: 10 },
    panels.cacheFetchDuration { gridPos: utils.gridPos(0, 24, 24, 8), id: 11, datasource: utils.datasource },
  ]),
  refresh: utils.refresh,
  revision: 1,
  schemaVersion: 39,
  tags: ['cache', 'redis', 'monitoring', 'performance'],
  // One datasource does not need a picker, and none of the panels above
  // filter by a template variable — #218 drops the dead `datasource` picker
  // and the `operation`/`result`/`error_type` label_values variables this
  // dashboard declared against `cache_operations_total`/`cache_errors_total`,
  // neither of which exists nor was ever referenced by a panel query.
  templating: { list: [] },
  time: {
    from: 'now-1h',
    to: 'now',
  },
  timepicker: {
    refresh_intervals: utils.refreshIntervals,
    time_options: ['5m', '15m', '1h', '6h', '12h', '24h', '2d', '7d', '30d'],
  },
  timezone: '',
  title: 'Redis Cache System Monitoring',
  uid: 'cache-monitoring-dashboard',
  version: 1,
  weekStart: '',
}
