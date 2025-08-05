{
  // Grid positioning utility
  gridPos(x, y, w, h): {
    x: x,
    y: y,
    w: w,
    h: h,
  },

  // Standard panel heights
  panelHeights: {
    stat: 4,
    timeseries: 8,
    table: 10,
    piechart: 6,
    row: 1,
  },

  // Color palettes for different metric types
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
        { color: $.colors.critical, value: null },
        { color: $.colors.warning, value: 0.9 },
        { color: $.colors.success, value: 0.95 },
      ],
    },
    latency: {
      mode: 'absolute',
      steps: [
        { color: $.colors.success, value: null },
        { color: $.colors.warning, value: 0.1 },
        { color: $.colors.critical, value: 0.5 },
      ],
    },
    errorCount: {
      mode: 'absolute',
      steps: [
        { color: $.colors.success, value: null },
        { color: $.colors.warning, value: 1 },
        { color: $.colors.critical, value: 5 },
      ],
    },
  },

  // Standard datasource configuration
  datasource: {
    type: 'prometheus',
    uid: '${DS_PROMETHEUS}',
  },

  // Row panel helper
  row(title, x, y, w=24): {
    collapsed: false,
    gridPos: $.gridPos(x, y, w, 1),  // Changed from self.gridPos to $.gridPos
    id: null,
    panels: [],
    title: title,
    type: 'row',
  },

  // Common field config for timeseries
  timeseriesFieldConfig: {
    defaults: {
      color: {
        mode: 'palette-classic',
      },
      custom: {
        axisLabel: '',
        axisPlacement: 'auto',
        barAlignment: 0,
        drawStyle: 'line',
        fillOpacity: 10,
        gradientMode: 'none',
        hideFrom: {
          legend: false,
          tooltip: false,
          vis: false,
        },
        lineInterpolation: 'linear',
        lineWidth: 1,
        pointSize: 5,
        scaleDistribution: {
          type: 'linear',
        },
        showPoints: 'never',
        spanNulls: false,
        stacking: {
          group: 'A',
          mode: 'none',
        },
        thresholdsStyle: {
          mode: 'off',
        },
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
    },
  },

  // Common field config for stat panels
  statFieldConfig: {
    defaults: {
      color: {
        mode: 'thresholds',
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
    },
  },

  // Common options for timeseries
  timeseriesOptions: {
    legend: {
      calcs: [],
      displayMode: 'list',
      placement: 'bottom',
    },
    tooltip: {
      mode: 'multi',
      sort: 'none',
    },
  },

  // Common options for stat panels
  statOptions: {
    colorMode: 'value',
    graphMode: 'area',
    justifyMode: 'auto',
    orientation: 'auto',
    reduceOptions: {
      calcs: ['lastNotNull'],
      fields: '',
      values: false,
    },
    textMode: 'auto',
  },

  // Helper for creating rate queries
  rateQuery(metric, labels='', interval='5m'): 
    'rate(' + metric + (if labels != '' then '{' + labels + '}' else '') + '[' + interval + '])',

  // Helper for creating histogram quantile queries
  histogramQuantile(quantile, metric, labels='', interval='5m', groupBy=''):
    'histogram_quantile(' + quantile + ', rate(' + metric + '_bucket' + 
    (if labels != '' then '{' + labels + '}' else '') + '[' + interval + '])' +
    (if groupBy != '' then ') by (' + groupBy + ')' else ''),

  // Helper for creating increase queries
  increaseQuery(metric, labels='', interval='5m'):
    'increase(' + metric + (if labels != '' then '{' + labels + '}' else '') + '[' + interval + '])',

  // Template variables
  templateVars: {
    instance: {
      name: 'instance',
      type: 'query',
      datasource: $.datasource,  // Changed from self.datasource to $.datasource
      query: 'label_values(ws_connections_total, instance)',
      refresh: 1,
      sort: 1,
      multi: true,
      includeAll: true,
      allValue: '.*',
    },
    event_type: {
      name: 'event_type',
      type: 'query',
      datasource: $.datasource,  // Changed from self.datasource to $.datasource
      query: 'label_values(ws_broadcast_operations_total, event_type)',
      refresh: 1,
      sort: 1,
      multi: true,
      includeAll: true,
      allValue: '.*',
    },
  },
}

