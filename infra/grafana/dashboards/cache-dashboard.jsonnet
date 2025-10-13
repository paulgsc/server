local panels = import 'lib/cache/cache-panels.libsonnet';
local utils = import 'lib/cache/cache-utils.libsonnet';

{
  annotations: {
    list: [
      {
        builtIn: 1,
        datasource: {
          type: 'datasource',
          uid: 'grafana',
        },
        enable: true,
        hide: true,
        iconColor: 'rgba(0, 211, 255, 1)',
        name: 'Annotations & Alerts',
        type: 'dashboard',
      },
    ],
  },
  description: 'Redis Cache System Monitoring Dashboard',
  editable: true,
  fiscalYearStartMonth: 0,
  graphTooltip: 1,
  id: null,
  links: [
    {
      asDropdown: false,
      icon: 'external link',
      includeVars: true,
      keepTime: true,
      tags: ['cache', 'redis', 'monitoring'],
      title: 'Cache Dashboards',
      type: 'dashboards',
    },
  ],
  liveNow: true,
  panels: [
    // Row 1: Cache Overview
    utils.row('Cache Operations Overview', 0, 0) { id: 1 },

    panels.cacheOperationsTotal {
      gridPos: utils.gridPos(0, 1, 4, 4),
      id: 2,
      datasource: utils.datasource,
    },

    panels.cacheSuccessRate {
      gridPos: utils.gridPos(4, 1, 4, 4),
      id: 3,
      datasource: utils.datasource,
    },

    panels.cacheHitRate {
      gridPos: utils.gridPos(8, 1, 4, 4),
      id: 4,
      datasource: utils.datasource,
    },

    panels.errorRate {
      gridPos: utils.gridPos(12, 1, 4, 4),
      id: 5,
      datasource: utils.datasource,
    },

    panels.compressionOperations {
      gridPos: utils.gridPos(16, 1, 4, 4),
      id: 6,
      datasource: utils.datasource,
    },

    panels.compressionRatio {
      gridPos: utils.gridPos(20, 1, 4, 4),
      id: 7,
      datasource: utils.datasource,
    },

    // Row 2: Cache Performance Metrics
    utils.row('Cache Performance Metrics', 0, 5) { id: 8 },

    panels.cacheOperationsByType {
      gridPos: utils.gridPos(0, 6, 12, 8),
      id: 9,
      datasource: utils.datasource,
    },

    panels.cacheHitsAndMisses {
      gridPos: utils.gridPos(12, 6, 12, 8),
      id: 10,
      datasource: utils.datasource,
    },

    // Row 3: Cache Hit/Miss Analysis
    utils.row('Cache Hit/Miss Analysis', 0, 14) { id: 11 },

    panels.cacheHitsByOperation {
      gridPos: utils.gridPos(0, 15, 24, 8),
      id: 12,
      datasource: utils.datasource,
    },

    // Row 4: Operation Duration Analysis
    utils.row('Operation Duration Analysis', 0, 23) { id: 13 },

    panels.cacheOperationDuration {
      gridPos: utils.gridPos(0, 24, 12, 8),
      id: 14,
      datasource: utils.datasource,
    },

    panels.redisConnectionDuration {
      gridPos: utils.gridPos(12, 24, 12, 8),
      id: 15,
      datasource: utils.datasource,
    },

    // Row 5: Compression Analysis
    utils.row('Compression Analysis', 0, 32) { id: 16 },

    panels.compressionDuration {
      gridPos: utils.gridPos(0, 33, 16, 8),
      id: 17,
      datasource: utils.datasource,
    },

    panels.dataSizeComparison {
      gridPos: utils.gridPos(16, 33, 8, 8),
      id: 18,
      datasource: utils.datasource,
    },

    // Row 6: Data Size Analysis
    utils.row('Data Size Analysis', 0, 41) { id: 19 },

    panels.cacheDataSize {
      gridPos: utils.gridPos(0, 42, 24, 8),
      id: 20,
      datasource: utils.datasource,
    },

    // Row 7: Error and Retry Analysis
    utils.row('Error and Retry Analysis', 0, 50) { id: 21 },

    panels.cacheErrors {
      gridPos: utils.gridPos(0, 51, 12, 8),
      id: 22,
      datasource: utils.datasource,
    },

    panels.cacheRetries {
      gridPos: utils.gridPos(12, 51, 12, 8),
      id: 23,
      datasource: utils.datasource,
    },

    // Row 8: TTL and Access Patterns
    utils.row('TTL and Access Patterns', 0, 59) { id: 24 },

    panels.cacheTTL {
      gridPos: utils.gridPos(0, 60, 8, 8),
      id: 25,
      datasource: utils.datasource,
    },

    panels.cacheAccessCount {
      gridPos: utils.gridPos(8, 60, 8, 8),
      id: 26,
      datasource: utils.datasource,
    },

    panels.cacheEntryAge {
      gridPos: utils.gridPos(16, 60, 8, 8),
      id: 27,
      datasource: utils.datasource,
    },

    // Row 9: Health Status Summary
    utils.row('Cache Health Status Summary', 0, 68) { id: 28 },

    panels.cacheHealthStatus {
      gridPos: utils.gridPos(0, 69, 24, 10),
      id: 29,
      datasource: utils.datasource,
    },
  ],
  refresh: utils.refresh,
  revision: 1,
  schemaVersion: 39,
  tags: ['cache', 'redis', 'monitoring', 'performance'],
  templating: {
    list: [
      {
        current: {
          selected: false,
          text: 'default',
          value: 'default',
        },
        description: 'Prometheus data source',
        critical: null,
        hide: 0,
        includeAll: false,
        label: 'Data Source',
        multi: false,
        name: 'datasource',
        options: [],
        query: 'prometheus',
        queryValue: '',
        refresh: 1,
        regex: '',
        skipUrlSync: false,
        type: 'datasource',
      },
      {
        current: {
          selected: false,
          text: 'All',
          value: '$__all',
        },
        description: 'Cache operation types to filter by',
        critical: null,
        hide: 0,
        includeAll: true,
        label: 'Operation',
        multi: true,
        name: 'operation',
        options: [],
        query: 'label_values(cache_operations_total, operation)',
        refresh: 2,
        regex: '',
        skipUrlSync: false,
        sort: 1,
        type: 'query',
      },
      {
        current: {
          selected: false,
          text: 'All',
          value: '$__all',
        },
        description: 'Cache operation results to filter by',
        critical: null,
        hide: 0,
        includeAll: true,
        label: 'Result',
        multi: true,
        name: 'result',
        options: [],
        query: 'label_values(cache_operations_total, result)',
        refresh: 2,
        regex: '',
        skipUrlSync: false,
        sort: 1,
        type: 'query',
      },
      {
        current: {
          selected: false,
          text: 'All',
          value: '$__all',
        },
        description: 'Error types to filter by',
        critical: null,
        hide: 0,
        includeAll: true,
        label: 'Error Type',
        multi: true,
        name: 'error_type',
        options: [],
        query: 'label_values(cache_errors_total, error_type)',
        refresh: 2,
        regex: '',
        skipUrlSync: false,
        sort: 1,
        type: 'query',
      },
      {
        current: {
          selected: false,
          text: '5m',
          value: '5m',
        },
        description: 'Rate calculation interval',
        critical: null,
        hide: 0,
        includeAll: false,
        label: 'Rate Interval',
        multi: false,
        name: 'rate_interval',
        options: [
          { selected: false, text: '1m', value: '1m' },
          { selected: true, text: '5m', value: '5m' },
          { selected: false, text: '10m', value: '10m' },
          { selected: false, text: '30m', value: '30m' },
        ],
        query: '1m,5m,10m,30m',
        queryValue: '',
        skipUrlSync: false,
        type: 'custom',
      },
    ],
  },
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
