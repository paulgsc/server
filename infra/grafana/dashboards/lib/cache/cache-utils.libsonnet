// Utility functions for cache monitoring dashboard
{
  // Grid position helper function
  gridPos(x, y, w, h): {
    x: x,
    y: y,
    w: w,
    h: h,
  },

  // Row panel helper
  row(title, x, y): {
    type: 'row',
    title: title,
    collapsed: false,
    gridPos: $.gridPos(x, y, 24, 1),
    panels: [],
  },

  // Common datasource configuration
  datasource: {
    type: 'prometheus',
    uid: 'prometheus',
  },

  // Common refresh settings
  refresh: '30s',
  refreshIntervals: ['5s', '10s', '30s', '1m', '5m', '15m', '30m', '1h', '2h', '1d'],

  // Time range presets
  timeRanges: [
    { from: 'now-5m', to: 'now', display: 'Last 5 minutes' },
    { from: 'now-15m', to: 'now', display: 'Last 15 minutes' },
    { from: 'now-30m', to: 'now', display: 'Last 30 minutes' },
    { from: 'now-1h', to: 'now', display: 'Last 1 hour' },
    { from: 'now-3h', to: 'now', display: 'Last 3 hours' },
    { from: 'now-6h', to: 'now', display: 'Last 6 hours' },
    { from: 'now-12h', to: 'now', display: 'Last 12 hours' },
    { from: 'now-24h', to: 'now', display: 'Last 24 hours' },
    { from: 'now-7d', to: 'now', display: 'Last 7 days' },
  ],

  // Common legend configurations
  legend: {
    table: {
      displayMode: 'table',
      placement: 'right',
      calcs: ['last', 'max', 'mean'],
    },
    list: {
      displayMode: 'list',
      placement: 'bottom',
      calcs: ['last', 'max'],
    },
    hidden: {
      displayMode: 'hidden',
    },
  },

  // Common tooltip configurations
  tooltip: {
    single: { mode: 'single', sort: 'none' },
    multi: { mode: 'multi', sort: 'desc' },
    multiAsc: { mode: 'multi', sort: 'asc' },
  },

  // Common color schemes
  colors: {
    success: 'green',
    warning: 'yellow',
    critical: 'red',
    info: 'blue',
    neutral: 'gray',
  },

  // Threshold configurations for different metric types
  thresholds: {
    successRate: {
      steps: [
        { color: $.colors.critical, value: null },
        { color: $.colors.warning, value: 95 },
        { color: $.colors.success, value: 99 },
      ],
    },
    errorRate: {
      steps: [
        { color: $.colors.success, value: null },
        { color: $.colors.warning, value: 1 },
        { color: $.colors.critical, value: 5 },
      ],
    },
    hitRate: {
      steps: [
        { color: $.colors.critical, value: null },
        { color: $.colors.warning, value: 70 },
        { color: $.colors.success, value: 90 },
      ],
    },
    compressionRatio: {
      steps: [
        { color: $.colors.critical, value: null },
        { color: $.colors.warning, value: 2 },
        { color: $.colors.success, value: 4 },
      ],
    },
    operationDuration: {
      steps: [
        { color: $.colors.success, value: null },
        { color: $.colors.warning, value: 0.1 },
        { color: $.colors.critical, value: 1.0 },
      ],
    },
    operations: {
      low: {
        steps: [
          { color: $.colors.success, value: null },
          { color: $.colors.warning, value: 100 },
          { color: $.colors.critical, value: 1000 },
        ],
      },
      high: {
        steps: [
          { color: $.colors.success, value: null },
          { color: $.colors.warning, value: 50 },
          { color: $.colors.critical, value: 200 },
        ],
      },
    },
    age: {
      steps: [
        { color: $.colors.success, value: null },
        { color: $.colors.warning, value: 3600 },  // 1 hour
        { color: $.colors.critical, value: 86400 },  // 1 day
      ],
    },
  },

  // Units for different metric types
  units: {
    seconds: 's',
    milliseconds: 'ms',
    bytes: 'bytes',
    percent: 'percent',
    reqps: 'reqps',
    ops: 'ops',
    short: 'short',
    none: 'none',
  },

  // Common field configurations
  fieldConfig: {
    stat: {
      defaults: {
        color: { mode: 'thresholds' },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            { color: $.colors.success, value: null },
          ],
        },
      },
    },
    timeseries: {
      defaults: {
        custom: {
          drawStyle: 'line',
          lineInterpolation: 'linear',
          lineWidth: 2,
          fillOpacity: 10,
          gradientMode: 'none',
          spanNulls: false,
          pointSize: 5,
          stacking: { mode: 'none', group: 'A' },
          axisPlacement: 'auto',
          axisLabel: '',
          scaleDistribution: { type: 'linear' },
          hideFrom: { legend: false, tooltip: false, vis: false },
        },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            { color: $.colors.success, value: null },
          ],
        },
      },
    },
    gauge: {
      defaults: {
        color: { mode: 'thresholds' },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            { color: $.colors.success, value: null },
          ],
        },
      },
    },
    bargauge: {
      defaults: {
        color: { mode: 'palette-classic' },
        custom: {
          displayMode: 'basic',
          orientation: 'horizontal',
        },
        mappings: [],
      },
    },
    table: {
      defaults: {
        custom: {
          align: 'auto',
          displayMode: 'auto',
        },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            { color: $.colors.success, value: null },
          ],
        },
      },
    },
  },

  // Common panel options
  options: {
    stat: {
      reduceOptions: {
        values: false,
        calcs: ['lastNotNull'],
        fields: '',
      },
      orientation: 'auto',
      textMode: 'auto',
      colorMode: 'value',
    },
    gauge: {
      reduceOptions: {
        values: false,
        calcs: ['lastNotNull'],
        fields: '',
      },
      orientation: 'auto',
      textMode: 'auto',
      colorMode: 'value',
      graphMode: 'area',
      justifyMode: 'auto',
    },
    bargauge: {
      reduceOptions: {
        values: false,
        calcs: ['lastNotNull'],
        fields: '',
      },
      orientation: 'horizontal',
      displayMode: 'basic',
    },
    timeseries: {
      legend: $.legend.table,
      tooltip: $.tooltip.multi,
    },
    table: {
      showHeader: true,
    },
  },

  // Common transformations
  transformations: {
    merge: {
      id: 'merge',
      options: {},
    },
    organize: {
      id: 'organize',
      options: {
        excludeByName: {
          Time: true,
        },
        indexByName: {},
        renameByName: {},
      },
    },
    calculateField: {
      id: 'calculateField',
      options: {
        mode: 'reduceRow',
        reduce: {
          reducer: 'lastNotNull',
        },
      },
    },
  },

  // Helper function to create a basic stat panel
  createStatPanel(title, query, unit, thresholds): {
    title: title,
    type: 'stat',
    targets: [
      {
        expr: query,
        legendFormat: title,
      },
    ],
    fieldConfig: {
      defaults: {
        unit: unit,
        color: { mode: 'thresholds' },
        thresholds: {
          mode: 'absolute',
          steps: thresholds,
        },
      },
    },
    options: $.options.stat,
  },

  // Helper function to create a basic timeseries panel
  createTimeseriesPanel(title, targets, unit, legend=$.legend.table): {
    title: title,
    type: 'timeseries',
    targets: targets,
    fieldConfig: {
      defaults: {
        unit: unit,
        custom: $.fieldConfig.timeseries.defaults.custom,
      },
    },
    options: {
      legend: legend,
      tooltip: $.tooltip.multi,
    },
  },

  // Helper function to create targets array
  createTargets(queries): [
    {
      expr: query.expr,
      legendFormat: query.legend,
    }
    for query in queries
  ],

  // Panel sizing presets
  panelSizes: {
    small: { w: 6, h: 8 },
    medium: { w: 8, h: 8 },
    large: { w: 12, h: 8 },
    wide: { w: 24, h: 8 },
    tall: { w: 12, h: 12 },
    stat: { w: 6, h: 4 },
    gauge: { w: 8, h: 8 },
    table: { w: 24, h: 10 },
  },

  // #219: no Grafana alert rules at this stage — the dashboard's visual
  // state is the alert. This used to hold four `condition:` strings against
  // `cache_operations_total`/`cache_operation_duration_seconds`/
  // `cache_retries_total`, none of which this workspace has ever emitted,
  // wired to no provisioning directory that would have loaded them.
}
