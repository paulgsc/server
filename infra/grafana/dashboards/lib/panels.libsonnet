local config = import 'config.libsonnet';
local utils = import 'utils.libsonnet';

{
  // The blackbox WS handshake probe's own round trip (infra/blackbox.yml's
  // `ws_handshake`). Its up/down is the "file_host /ws" lane of the SERVICES
  // timeline (overview-panels.libsonnet); this is the how-long, for when that
  // lane flickers. The panels that used to live here — a combined TCP×HTTP
  // uptime stat, a 30-day SLA, a 7-day trend — were built on the old
  // `http_2xx` probe, which could never succeed (see blackbox.yml), and are
  // replaced by AVAILABILITY and the SERVICES timeline.
  probeDuration: {
    datasource: config.prometheusDataSource,
    description: 'How long the blackbox WebSocket handshake probe takes, TCP connect through the 101 response. Normally a few milliseconds; the probe fails at 5s.',
    fieldConfig: utils.timeSeriesFieldConfig('s', 1),
    options: utils.timeSeriesOptions,
    targets: [
      { expr: 'probe_duration_seconds{job="websocket_blackbox_http"}', legendFormat: 'ws handshake', refId: 'A' },
      { expr: 'probe_duration_seconds{job="websocket_blackbox_tcp"}', legendFormat: 'tcp connect', refId: 'B' },
    ],
    title: 'WS Handshake Probe Duration',
    type: 'timeseries',
  },

  // =============== APPLICATION METRICS ===============
  //
  // #212/#217: this used to also carry an HTTP request-rate/latency row and
  // a cache row. Both queried `http_requests_total*` and
  // `cache_operations_total*` — names no crate in this workspace has ever
  // emitted (no `axum-prometheus` layer, no manual counter for either
  // family). Rate limiting and per-namespace cache detail get their own
  // dashboards once real metrics back them (parked `rate-limit.jsonnet` /
  // rebuilt `cache-dashboard.jsonnet`); this one keeps only what
  // `file_host` actually records today.

  operationDuration: {
    datasource: config.prometheusDataSource,
    fieldConfig: utils.timeSeriesFieldConfig('s', 1),
    id: 9,
    options: utils.timeSeriesOptions,
    targets: [
      {
        expr: 'histogram_quantile(0.95, sum(rate(operation_duration_seconds_bucket[5m])) by (le, handler, operation))',
        legendFormat: '{{handler}} - {{operation}}',
        refId: 'A',
      },
    ],
    title: '⚙️ Operation Duration (P95)',
    type: 'timeseries',
  },

  // Every ERROR-level tracing event, workspace-wide, broken down by the
  // module (`target`) that logged it — the general counterpart to nudge's
  // configErrors (nudge-panels.libsonnet). `ErrorEventMetricsLayer`
  // (metrics/observability.rs) mirrors every ERROR event into
  // `tracing_events_total` rather than requiring each call site to
  // remember to record its own counter, so a fault in a WS handler, a REST
  // route, or a background task nobody's gotten around to instrumenting
  // yet still shows up here. Routine 4xx responses log at WARN
  // (error.rs's `into_response`), not ERROR, so this reflects real
  // operational faults rather than ordinary client traffic — see
  // infra/grafana/provisioning/alerting/file-host-errors.yml, which alerts
  // on exactly this series.
  // `increase(...[5m])` is a count per five minutes, not a rate — 'short'
  // and the title say so. Under 'ops' a waker failing once per 300s tick
  // read as "1 ops/s", three hundred times what was happening.
  tracingErrors: {
    datasource: config.prometheusDataSource,
    fieldConfig: utils.timeSeriesFieldConfig('short', 0),
    id: 10,
    options: utils.timeSeriesOptions,
    targets: [{
      expr: 'sum by (target) (increase(tracing_events_total{level="error"}[5m]))',
      legendFormat: '{{target}}',
      refId: 'A',
    }],
    title: '🚨 Tracing Error Events by Target (per 5m)',
    type: 'timeseries',
  },
}
