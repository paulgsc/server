local config = import 'config.libsonnet';
local utils = import 'utils.libsonnet';

{
  uptimeOverallStatus: {
    datasource: config.prometheusDataSource,
    fieldConfig: {
      defaults: {
        color: { mode: 'thresholds' },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            { color: 'red', value: 0 },
            { color: 'green', value: 1 },
          ],
        },
        unit: 'none',
      },
    },
    id: 1,
    options: utils.statOptions { graphMode: 'none' },
    pluginVersion: '9.2.0',
    targets: [
      {
        datasource: config.prometheusDataSource,
        expr: 'probe_success{job="websocket_blackbox_tcp"} * on(service) probe_success{job="websocket_blackbox_http"}',
        instant: true,
        refId: 'A',
      },
    ],
    title: '✅ Overall Uptime Status',
    type: 'stat',
  },

  uptimeSLA30d: {
    datasource: config.prometheusDataSource,
    fieldConfig: {
      defaults: {
        color: { mode: 'thresholds' },
        decimals: 3,
        thresholds: {
          mode: 'absolute',
          steps: [
            { color: 'red', value: 0 },
            { color: 'orange', value: 99.0 },
            { color: 'yellow', value: 99.9 },
            { color: 'green', value: 99.95 },
          ],
        },
        unit: 'percent',
      },
    },
    id: 2,
    options: utils.statOptions { graphMode: 'area' },
    pluginVersion: '9.2.0',
    targets: [
      {
        datasource: config.prometheusDataSource,
        expr: 'avg_over_time((probe_success{job="websocket_blackbox_tcp"} * on(service)  probe_success{job="websocket_blackbox_http"})[30d:]) * 100',
        instant: true,
        refId: 'A',
      },
    ],
    title: '🎯 30-Day Uptime SLA',
    type: 'stat',
  },

  uptimeTrend7d: {
    datasource: config.prometheusDataSource,
    fieldConfig: utils.timeSeriesFieldConfig('percent', 99.9),
    id: 3,
    options: utils.timeSeriesOptions,
    targets: [
      {
        datasource: config.prometheusDataSource,
        expr: 'avg_over_time((probe_success{job="websocket_blackbox_tcp"} * on(service) probe_success{job="websocket_blackbox_http"})[1h:]) * 100',
        legendFormat: 'Combined Uptime %',
        range: true,
        refId: 'A',
      },
    ],
    title: '📈 7-Day Uptime Trend',
    type: 'timeseries',
  },

  // =============== DIAGNOSTIC PANELS ===============

  tcpConnectivity: {
    datasource: config.prometheusDataSource,
    fieldConfig: {
      defaults: {
        color: { mode: 'thresholds' },
        thresholds: {
          mode: 'absolute',
          steps: [
            { color: 'red', value: 0 },
            { color: 'green', value: 1 },
          ],
        },
        unit: 'none',
      },
    },
    id: 4,
    options: utils.statOptions { graphMode: 'none' },
    targets: [
      {
        expr: 'probe_success{job="websocket_blackbox_tcp"}',
        instant: true,
        refId: 'A',
      },
    ],
    title: '🔌 TCP Connectivity (Port 3000)',
    type: 'stat',
  },

  httpWebSocketProbe: {
    datasource: config.prometheusDataSource,
    fieldConfig: {
      defaults: {
        color: { mode: 'thresholds' },
        thresholds: {
          mode: 'absolute',
          steps: [
            { color: 'red', value: 0 },
            { color: 'green', value: 1 },
          ],
        },
        unit: 'none',
      },
    },
    id: 5,
    options: utils.statOptions { graphMode: 'none' },
    targets: [
      {
        expr: 'probe_success{job="websocket_blackbox_http"}',
        instant: true,
        refId: 'A',
      },
    ],
    title: '💬 HTTP /ws Probe (101/429/503)',
    type: 'stat',
  },

  probeDiagnostics: {
    datasource: config.prometheusDataSource,
    fieldConfig: {
      defaults: {
        unit: 's',
        decimals: 3,
        color: { mode: 'palette-classic' },
      },
      overrides: [
        {
          matcher: { id: 'byRegexp', options: '.*Failures.*' },
          properties: [
            { id: 'unit', value: 'short' },
            { id: 'custom.axisPlacement', value: 'right' },
          ],
        },
      ],
    },
    id: 6,
    options: utils.timeSeriesOptions {
      legend: { showLegend: true },
    },
    targets: [
      {
        expr: 'probe_duration_seconds{job="websocket_blackbox_tcp"}',
        legendFormat: 'TCP P90 Duration',
        refId: 'A',
      },
      {
        expr: 'probe_duration_seconds{job="websocket_blackbox_http"}',
        legendFormat: 'HTTP P90 Duration',
        refId: 'B',
      },
      {
        expr: 'sum(rate(probe_success{job="websocket_blackbox_tcp"} == 0)[5m:])',
        legendFormat: 'TCP Failures',
        refId: 'C',
      },
      {
        expr: 'sum(rate(probe_success{job="websocket_blackbox_http"} == 0)[5m:])',
        legendFormat: 'HTTP Failures',
        refId: 'D',
      },
    ],
    title: '🔍 Probe Diagnostics (Duration & Failures)',
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
  tracingErrors: {
    datasource: config.prometheusDataSource,
    fieldConfig: utils.timeSeriesFieldConfig('ops', 0),
    id: 10,
    options: utils.timeSeriesOptions,
    targets: [{
      expr: 'sum by (target) (increase(tracing_events_total{level="error"}[5m]))',
      legendFormat: '{{target}}',
      refId: 'A',
    }],
    title: '🚨 Tracing Error Events by Target',
    type: 'timeseries',
  },
}
