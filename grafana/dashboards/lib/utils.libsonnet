{
  // Grid position helper
  gridPos(x, y, w, h): {
    h: h,
    w: w,
    x: x,
    y: y,
  },

  // Common field config for time series panels
  timeSeriesFieldConfig(unit, redThreshold): {
    defaults: {
      color: {
        mode: 'palette-classic',
      },
      custom: {
        axisLabel: '',
        axisPlacement: 'auto',
        barAlignment: 0,
        drawStyle: 'line',
        fillOpacity: 20,
        gradientMode: 'none',
        hideFrom: {
          legend: false,
          tooltip: false,
          viz: false,
        },
        lineInterpolation: 'linear',
        lineWidth: 2,
        pointSize: 5,
        scaleDistribution: {
          type: 'linear',
        },
        showPoints: 'auto',
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
          {
            color: 'red',
            value: redThreshold,
          },
        ],
      },
      unit: unit,
    },
    overrides: [],
  },

  // Common options for time series panels
  timeSeriesOptions: {
    legend: {
      calcs: [],
      displayMode: 'list',
      placement: 'bottom',
      showLegend: true,
    },
    tooltip: {
      mode: 'multi',
      sort: 'desc',
    },
  },

  // Common field config for stat panels
  statFieldConfig(unit, color, redThreshold): {
    defaults: {
      color: {
        fixedColor: color,
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
          {
            color: 'red',
            value: redThreshold,
          },
        ],
      },
      unit: unit,
    },
    overrides: [],
  },

  // Common options for stat panels
  statOptions: {
    colorMode: 'value',
    graphMode: 'area',
    justifyMode: 'auto',
    orientation: 'auto',
    reduceOptions: {
      calcs: ['last'],
      fields: '',
      values: false,
    },
    textMode: 'auto',
  },
}
