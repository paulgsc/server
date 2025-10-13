// utils.libsonnet
// Shared utilities for Grafana dashboards with Jsonnet
// Use: local utils = import 'utils.libsonnet';

local utils = {
  // Grid positioning utility
  gridPos(x, y, w, h):: {
    x: x,
    y: y,
    w: w,
    h: h,
  },

  // Row helper for dashboard organization
  row(title, x, y):: {
    datasource: {
      type: 'datasource',
      uid: 'ws_grafana',
    },
    gridPos: { x: x, y: y, w: 24, h: 1 },
    id: null,
    title: title,
    type: 'row',
  },

  // Standard panel heights
  panelHeights: {
    stat: 3,
    timeseries: 5,
    table: 8,
    piechart: 5,
  },

  // Color palettes
  colors: {
    success: '#73BF69',
    warning: '#FF9830',
    critical: '#F2495C',
    info: '#5794F2',
    neutral: '#8AB8FF',
  },

  // Common thresholds
  thresholds: {
    successRate: {
      mode: 'absolute',
      steps: [
        { value: null, color: utils.colors.critical },
        { value: 0.9, color: utils.colors.warning },
        { value: 0.95, color: utils.colors.success },
      ],
    },
    latency: {
      mode: 'absolute',
      steps: [
        { value: null, color: utils.colors.success },
        { value: 0.1, color: utils.colors.warning },
        { value: 0.5, color: utils.colors.critical },
      ],
    },
    errorCount: {
      mode: 'absolute',
      steps: [
        { value: null, color: utils.colors.success },
        { value: 1, color: utils.colors.warning },
        { value: 5, color: utils.colors.critical },
      ],
    },
  },

  // Default datasource (can be overridden)
  datasource: {
    type: 'prometheus',
    uid: '$datasource',  // Grafana template var
  },

  // === Field Config Templates ===
  fieldConfig: {
    // Base for all time series
    timeseries: {
      defaults: {
        color: { mode: 'palette-classic' },
        custom: {
          axisLabel: '',
          axisPlacement: 'auto',
          barAlignment: 0,
          drawStyle: 'line',
          fillOpacity: 10,
          gradientMode: 'none',
          hideFrom: { legend: false, tooltip: false, viz: false },
          lineInterpolation: 'linear',
          lineWidth: 1,
          pointSize: 5,
          scaleDistribution: { type: 'linear' },
          showPoints: 'never',
          spanNulls: false,
          stacking: { group: 'A', mode: 'none' },
          thresholdsStyle: { mode: 'off' },
        },
        mappings: [],
        thresholds: { mode: 'absolute', steps: [{ value: null, color: 'green' }] },
      },
    },

    // Base for stat panels
    stat: {
      defaults: {
        color: { mode: 'thresholds' },
        mappings: [],
        thresholds: { mode: 'absolute', steps: [{ value: null, color: 'green' }] },
        unit: 'short',
      },
    },
  },

  // === Display Options ===
  options: {
    timeseries: {
      legend: { calcs: [], displayMode: 'list', placement: 'bottom' },
      tooltip: { mode: 'multi', sort: 'none' },
    },
    stat: {
      colorMode: 'value',
      graphMode: 'none',
      justifyMode: 'auto',
      orientation: 'auto',
      reduceOptions: { calcs: ['lastNotNull'], fields: '', values: false },
      textMode: 'auto',
    },
    table: {
      showHeader: true,
      displayMode: 'auto',
    },
  },

  // === Query Helpers (with cluster/job filtering) ===
  // Common filter for all queries
  local commonFilter = 'cluster=~"$cluster",job=~"$job"',

  // rate(metric, interval="5m", extraLabels="")
  rate(metric, interval='5m', extraLabels='')::
    'rate(' + metric + '{' + commonFilter +
    (if extraLabels != '' then ', ' + extraLabels else '') + '}[' + interval + '])',

  // increase(metric, interval="1h", extraLabels="")
  increase(metric, interval='1h', extraLabels='')::
    'increase(' + metric + '{' + commonFilter +
    (if extraLabels != '' then ', ' + extraLabels else '') + '}[' + interval + '])',

  // histogramQuantile(0.95, "ws_message_duration_seconds", 'type="subscribe"', 'by(type)')
  histogramQuantile(quantile, metric, extraLabels='', groupBy='')::
    'histogram_quantile(' + quantile + ', ' +
    'sum(rate(' + metric + '_bucket{' + commonFilter +
    (if extraLabels != '' then ', ' + extraLabels else '') + '}[5m])) ' +
    'by (le' + (if groupBy != '' then ', ' + groupBy else '') + '))',

  // sum(rate(...)) by(labels)
  sumRate(metric, extraLabels='', byLabels='')::
    'sum(' + self.rate(metric, '5m', extraLabels) + ')' +
    (if byLabels != '' then ' by (' + byLabels + ')' else ''),

  // topk(5, increase(...))
  topK(k, metric, interval='1h', extraLabels='')::
    'topk(' + k + ', ' + self.increase(metric, interval, extraLabels) + ')',

  // === Template Variables ===
  templateVars: {
    cluster: {
      name: 'cluster',
      type: 'query',
      dataSource: utils.datasource,
      query: 'label_values(up, cluster)',
      refresh: 1,
      includeAll: true,
      multi: true,
      allValue: '.*',
      label: 'Cluster',
    },
    job: {
      name: 'job',
      type: 'query',
      dataSource: utils.datasource,
      query: 'label_values(up{cluster=~"$cluster"}, job)',
      refresh: 1,
      includeAll: true,
      multi: true,
      allValue: '.*',
      label: 'Job',
    },
    instance: {
      name: 'instance',
      type: 'query',
      dataSource: utils.datasource,
      query: 'label_values(ws_connections_total, instance)',
      refresh: 1,
      includeAll: true,
      multi: true,
      allValue: '.*',
      label: 'Instance',
    },
    event_type: {
      name: 'event_type',
      type: 'query',
      dataSource: utils.datasource,
      query: 'label_values(ws_broadcast_operations_total, event_type)',
      refresh: 1,
      includeAll: true,
      multi: true,
      allValue: '.*',
      label: 'Event Type',
    },
    interval: {
      name: 'interval',
      type: 'interval',
      label: 'Resolution',
      default: '1m',
      options: ['10s', '30s', '1m', '5m', '10m', '30m', '1h'],
    },
  },
};

utils
