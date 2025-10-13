{
  gridPos(x, y, w, h): { x: x, y: y, w: w, h: h },

  timeSeriesFieldConfig(unit, redThreshold): {
    defaults: {
      color: { mode: 'palette-classic' },
      custom: {
        drawStyle: 'line',
        fillOpacity: 15,
        lineWidth: 2,
        pointSize: 4,
        showPoints: 'never',
        spanNulls: true,
        stacking: { mode: 'none' },
      },
      thresholds: {
        mode: 'absolute',
        steps: [
          { color: 'green', value: null },
          { color: 'red', value: redThreshold },
        ],
      },
      unit: unit,
    },
  },

  timeSeriesOptions: {
    legend: { showLegend: true, placement: 'bottom' },
    tooltip: { mode: 'multi', sort: 'desc' },
  },

  statFieldConfig(unit, color, redThreshold): {
    defaults: {
      color: { fixedColor: color, mode: 'fixed' },
      thresholds: {
        mode: 'absolute',
        steps: [
          { color: 'green', value: null },
          { color: 'red', value: redThreshold },
        ],
      },
      unit: unit,
    },
  },

  statOptions: {
    colorMode: 'value',
    graphMode: 'area',
    justifyMode: 'center',
    orientation: 'auto',
    reduceOptions: { calcs: ['last'], values: false },
    textMode: 'auto',
  },
}
