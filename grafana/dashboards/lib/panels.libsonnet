local config = import 'config.libsonnet';
local utils = import 'utils.libsonnet';

{
  httpRequestRate: {
    datasource: config.prometheusDataSource,
    fieldConfig: utils.timeSeriesFieldConfig('reqps', 80),
    id: 2,
    options: utils.timeSeriesOptions,
    targets: [
      {
        datasource: config.prometheusDataSource,
        editorMode: 'builder',
        expr: 'rate(http_requests_total[1m])',
        legendFormat: '{{method}} {{route}} → {{status}}',
        range: true,
        refId: 'A',
      },
    ],
    title: 'HTTP Request Rate (RPS)',
    type: 'timeseries',
  },

  httpLatency: {
    datasource: config.prometheusDataSource,
    fieldConfig: utils.timeSeriesFieldConfig('s', 1),
    id: 3,
    options: utils.timeSeriesOptions,
    targets: [
      {
        datasource: config.prometheusDataSource,
        editorMode: 'builder',
        expr: 'histogram_quantile(0.50, sum(rate(http_request_duration_seconds_bucket[5m])) by (le, method, route))',
        legendFormat: 'p50 - {{method}} {{route}}',
        range: true,
        refId: 'A',
      },
      {
        datasource: config.prometheusDataSource,
        editorMode: 'builder',
        expr: 'histogram_quantile(0.90, sum(rate(http_request_duration_seconds_bucket[5m])) by (le, method, route))',
        legendFormat: 'p90 - {{method}} {{route}}',
        range: true,
        refId: 'B',
      },
      {
        datasource: config.prometheusDataSource,
        editorMode: 'builder',
        expr: 'histogram_quantile(0.99, sum(rate(http_request_duration_seconds_bucket[5m])) by (le, method, route))',
        legendFormat: 'p99 - {{method}} {{route}}',
        range: true,
        refId: 'C',
      },
    ],
    title: 'HTTP Latency (P50/P90/P99)',
    type: 'timeseries',
  },

  operationDuration: {
    datasource: config.prometheusDataSource,
    fieldConfig: utils.timeSeriesFieldConfig('s', 1),
    id: 4,
    options: utils.timeSeriesOptions,
    targets: [
      {
        datasource: config.prometheusDataSource,
        editorMode: 'builder',
        expr: 'histogram_quantile(0.95, sum(rate(operation_duration_seconds_bucket[5m])) by (le, handler, operation))',
        legendFormat: '{{handler}} - {{operation}}',
        range: true,
        refId: 'A',
      },
    ],
    title: 'Operation Duration (P95 by Handler)',
    type: 'timeseries',
  },

  cacheHitsMisses: {
    datasource: config.prometheusDataSource,
    fieldConfig: utils.timeSeriesFieldConfig('none', 1),
    id: 5,
    options: utils.timeSeriesOptions,
    targets: [
      {
        datasource: config.prometheusDataSource,
        editorMode: 'builder',
        expr: 'sum(cache_operations_total{result="hit"}) by (handler)',
        legendFormat: '{{handler}} - Hit',
        refId: 'A',
      },
      {
        datasource: config.prometheusDataSource,
        editorMode: 'builder',
        expr: 'sum(cache_operations_total{result="miss"}) by (handler)',
        legendFormat: '{{handler}} - Miss',
        refId: 'B',
      },
    ],
    title: 'Cache Hits vs Misses by Handler',
    type: 'timeseries',
  },

  totalRequests: {
    datasource: config.prometheusDataSource,
    fieldConfig: utils.statFieldConfig('none', 'semi-dark-orange', 100),
    id: 6,
    options: utils.statOptions,
    pluginVersion: '9.2.0',
    targets: [
      {
        datasource: config.prometheusDataSource,
        editorMode: 'builder',
        expr: 'sum(http_requests_total)',
        instant: true,
        refId: 'A',
      },
    ],
    title: 'Total Requests',
    type: 'stat',
  },

  totalCacheOps: {
    datasource: config.prometheusDataSource,
    fieldConfig: utils.statFieldConfig('ops', 'dark-purple', 50),
    id: 7,
    options: utils.statOptions,
    pluginVersion: '9.2.0',
    targets: [
      {
        datasource: config.prometheusDataSource,
        editorMode: 'builder',
        expr: 'sum(cache_operations_total)',
        instant: true,
        refId: 'A',
      },
    ],
    title: 'Total Cache Ops',
    type: 'stat',
  },

  cacheHitRate: {
    datasource: config.prometheusDataSource,
    fieldConfig: {
      defaults: {
        color: {
          mode: 'thresholds',
        },
        mappings: [],
        thresholds: {
          mode: 'percentage',
          steps: [
            {
              color: 'green',
              value: null,
            },
            {
              color: 'orange',
              value: 70,
            },
            {
              color: 'red',
              value: 90,
            },
          ],
        },
        unit: 'percent',
      },
      overrides: [],
    },
    id: 8,
    options: utils.statOptions { graphMode: 'none' },
    pluginVersion: '9.2.0',
    targets: [
      {
        datasource: config.prometheusDataSource,
        editorMode: 'builder',
        expr: 'sum(cache_operations_total{result="hit"}) / sum(cache_operations_total) * 100',
        instant: true,
        refId: 'A',
      },
    ],
    title: 'Cache Hit Rate',
    type: 'stat',
  },

  rateLimitedRequests: {
    datasource: config.prometheusDataSource,
    fieldConfig: {
      defaults: {
        color: {
          fixedColor: 'semi-dark-blue',
          mode: 'fixed',
        },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
          ],
        },
        unit: 'none',
      },
      overrides: [],
    },
    id: 9,
    options: utils.statOptions,
    pluginVersion: '9.2.0',
    targets: [
      {
        datasource: config.prometheusDataSource,
        editorMode: 'builder',
        expr: 'sum(http_requests_total{status="429"})',
        instant: true,
        refId: 'A',
      },
    ],
    title: 'Rate Limited Requests (429)',
    type: 'stat',
  },
}
