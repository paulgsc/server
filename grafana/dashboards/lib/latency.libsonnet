local grafana = import 'grafonnet/grafana.libsonnet';
local timeSeries = grafana.timeSeries;
local table = grafana.table;
local heatmap = grafana.heatmap;
local prometheus = grafana.prometheus;

{
  latencyPercentiles:: timeSeries.new(
    title='Request Latency Percentiles',
    datasource='${DS_PROMETHEUS}',
  )
                       .addTargets([
    prometheus.new(
      expr='histogram_quantile(0.50, rate(operation_duration_seconds_bucket{handler=~"$handler"}[5m]))',
      legendFormat='{{handler}} p50',
    ),
    prometheus.new(
      expr='histogram_quantile(0.90, rate(operation_duration_seconds_bucket{handler=~"$handler"}[5m]))',
      legendFormat='{{handler}} p90',
    ),
    prometheus.new(
      expr='histogram_quantile(0.95, rate(operation_duration_seconds_bucket{handler=~"$handler"}[5m]))',
      legendFormat='{{handler}} p95',
    ),
    prometheus.new(
      expr='histogram_quantile(0.99, rate(operation_duration_seconds_bucket{handler=~"$handler"}[5m]))',
      legendFormat='{{handler}} p99',
    ),
  ])
                       .standardOptions.withUnit('s')
                       .fieldConfig.defaults.custom.withDrawStyle('line')
                       .fieldConfig.defaults.custom.withLineWidth(2)
                       .fieldConfig.defaults.custom.withFillOpacity(10)
                       .fieldConfig.defaults.custom.withLineInterpolation('linear')
                       .fieldConfig.defaults.custom.withShowPoints('never')
                       .fieldConfig.defaults.custom.withSpanNulls(false)
                       .addThresholds([
    { color: 'green', value: null },
    { color: 'yellow', value: 0.1 },
    { color: 'red', value: 0.5 },
  ])
                       .options.legend.withDisplayMode('table')
                       .options.legend.withPlacement('right')
                       .options.legend.withCalcs(['last', 'max'])
                       .options.tooltip.withMode('multi')
                       .options.tooltip.withSort('desc'),

  topLatencyRequests:: table.new(
    title='Top 10 Slowest Operations (p95)',
    datasource='${DS_PROMETHEUS}',
  )
                       .addTarget(
    prometheus.new(
      expr='topk(10, histogram_quantile(0.95, rate(operation_duration_seconds_bucket{handler=~"$handler"}[5m])))',
      format='table',
      instant=true,
    )
  )
                       .standardOptions.withUnit('s')
                       .addThresholds([
    { color: 'green', value: null },
    { color: 'yellow', value: 0.1 },
    { color: 'red', value: 0.5 },
  ])
                       .addTransformations([
    {
      id: 'organize',
      options: {
        excludeByName: {
          Time: true,
          __name__: true,
          job: true,
          instance: true,
        },
        renameByName: {
          Value: 'Latency (s)',
          handler: 'Handler',
          operation: 'Operation',
        },
      },
    },
  ]),

  operationDurationHeatmap:: heatmap.new(
    title='Request Duration Heatmap',
    datasource='${DS_PROMETHEUS}',
  )
                             .addTarget(
    prometheus.new(
      expr='rate(operation_duration_seconds_bucket{handler=~"$handler"}[5m])',
      format='heatmap',
      legendFormat='{{le}}',
    )
  )
                             .options.color.withExponent(0.5)
                             .options.color.withFill('dark-orange')
                             .options.color.withMode('spectrum')
                             .options.color.withReverse(false)
                             .options.color.withScale('exponential')
                             .options.color.withScheme('Spectral')
                             .options.color.withSteps(64)
                             .options.withCellGap(2)
                             .options.withCalculate(false)
                             .options.yAxis.withUnit('s')
                             .options.yAxis.withAxisPlacement('left')
                             .options.yAxis.withReverse(false)
                             .options.tooltip.withShow(true)
                             .options.tooltip.withYHistogram(false)
                             .options.legend.withShow(true)
                             .options.filterValues.withLe(1e-9),
}
