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
        expr: 'probe_success{job="websocket_blackbox_tcp"} * probe_success{job="websocket_blackbox_http"}',
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
        expr: 'avg_over_time((probe_success{job="websocket_blackbox_tcp"} * probe_success{job="websocket_blackbox_http"})[30d:]) * 100',
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
        expr: 'avg_over_time((probe_success{job="websocket_blackbox_tcp"} * probe_success{job="websocket_blackbox_http"})[1h:]) * 100',
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
        expr: 'histogram_quantile(0.90, sum(rate(probe_duration_seconds_bucket{job="websocket_blackbox_tcp"}[5m])) by (le))',
        legendFormat: 'TCP P90 Duration',
        refId: 'A',
      },
      {
        expr: 'histogram_quantile(0.90, sum(rate(probe_duration_seconds_bucket{job="websocket_blackbox_http"}[5m])) by (le))',
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

  // =============== EXISTING APPLICATION METRICS ===============

  httpRequestRate: {
    datasource: config.prometheusDataSource,
    fieldConfig: utils.timeSeriesFieldConfig('reqps', 80),
    id: 7,
    options: utils.timeSeriesOptions,
    targets: [
      {
        datasource: config.prometheusDataSource,
        expr: 'rate(http_requests_total[1m])',
        legendFormat: '{{method}} {{route}} → {{status}}',
        range: true,
        refId: 'A',
      },
    ],
    title: '📨 HTTP Request Rate (RPS)',
    type: 'timeseries',
  },

  httpLatency: {
    datasource: config.prometheusDataSource,
    fieldConfig: utils.timeSeriesFieldConfig('s', 1),
    id: 8,
    options: utils.timeSeriesOptions,
    targets: [
      {
        expr: 'histogram_quantile(0.50, sum(rate(http_request_duration_seconds_bucket[5m])) by (le, method, route))',
        legendFormat: 'p50 - {{method}} {{route}}',
        refId: 'A',
      },
      {
        expr: 'histogram_quantile(0.90, sum(rate(http_request_duration_seconds_bucket[5m])) by (le, method, route))',
        legendFormat: 'p90 - {{method}} {{route}}',
        refId: 'B',
      },
      {
        expr: 'histogram_quantile(0.99, sum(rate(http_request_duration_seconds_bucket[5m])) by (le, method, route))',
        legendFormat: 'p99 - {{method}} {{route}}',
        refId: 'C',
      },
    ],
    title: '⏱️ HTTP Latency (P50/P90/P99)',
    type: 'timeseries',
  },

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

  cacheHitsMisses: {
    datasource: config.prometheusDataSource,
    fieldConfig: utils.timeSeriesFieldConfig('none', 1),
    id: 10,
    options: utils.timeSeriesOptions,
    targets: [
      {
        expr: 'sum(cache_operations_total{result="hit"}) by (handler)',
        legendFormat: '{{handler}} - Hit',
        refId: 'A',
      },
      {
        expr: 'sum(cache_operations_total{result="miss"}) by (handler)',
        legendFormat: '{{handler}} - Miss',
        refId: 'B',
      },
    ],
    title: '📦 Cache Hits vs Misses',
    type: 'timeseries',
  },

  totalRequests: {
    datasource: config.prometheusDataSource,
    fieldConfig: utils.statFieldConfig('none', 'semi-dark-orange', 100),
    id: 11,
    options: utils.statOptions,
    targets: [
      {
        expr: 'sum(http_requests_total)',
        instant: true,
        refId: 'A',
      },
    ],
    title: '🔢 Total Requests',
    type: 'stat',
  },

  totalCacheOps: {
    datasource: config.prometheusDataSource,
    fieldConfig: utils.statFieldConfig('ops', 'dark-purple', 50),
    id: 12,
    options: utils.statOptions,
    targets: [
      {
        expr: 'sum(cache_operations_total)',
        instant: true,
        refId: 'A',
      },
    ],
    title: '🧮 Total Cache Ops',
    type: 'stat',
  },

  cacheHitRate: {
    datasource: config.prometheusDataSource,
    fieldConfig: {
      defaults: {
        color: { mode: 'thresholds' },
        thresholds: {
          mode: 'absolute',
          steps: [
            { color: 'red', value: 0 },
            { color: 'orange', value: 70 },
            { color: 'green', value: 90 },
          ],
        },
        unit: 'percent',
      },
    },
    id: 13,
    options: utils.statOptions { graphMode: 'none' },
    targets: [
      {
        expr: 'sum(cache_operations_total{result="hit"}) / sum(cache_operations_total) * 100',
        instant: true,
        refId: 'A',
      },
    ],
    title: '🎯 Cache Hit Rate',
    type: 'stat',
  },

  rateLimitedRequests: {
    datasource: config.prometheusDataSource,
    fieldConfig: {
      defaults: {
        color: { fixedColor: 'semi-dark-blue', mode: 'fixed' },
        unit: 'short',
      },
    },
    id: 14,
    options: utils.statOptions,
    targets: [
      {
        expr: 'sum(http_requests_total{status="429"})',
        instant: true,
        refId: 'A',
      },
    ],
    title: '🚦 Rate Limited (429)',
    type: 'stat',
  },
}
