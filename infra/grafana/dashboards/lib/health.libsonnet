local grafana = import 'grafonnet/grafana.libsonnet';
local table = grafana.table;
local timeSeries = grafana.timeSeries;
local prometheus = grafana.prometheus;

{
  serviceHealthStatus:: table.new(
    title='Service Health Status',
    datasource='${DS_PROMETHEUS}',
  )
                        .addTarget(
    prometheus.new(
      expr='up',
      format='table',
      instant=true,
    )
  )
                        .addThresholds([
    { color: 'red', value: null },
    { color: 'green', value: 1 },
  ])
                        .setMappings([
    {
      options: {
        '0': { color: 'red', index: 0, text: 'DOWN' },
        '1': { color: 'green', index: 1, text: 'UP' },
      },
      type: 'value',
    },
  ]),

  requestsPerSecond:: timeSeries.new(
    title='Requests Per Second',
    datasource='${DS_PROMETHEUS}',
  )
                      .addTarget(
    prometheus.new(
      expr='rate(operation_duration_seconds_count{handler=~"$handler"}[5m])',
      legendFormat='{{handler}} - {{operation}}',
    )
  )
                      .standardOptions.withUnit('reqps')
                      .fieldConfig.defaults.custom.withDrawStyle('line')
                      .fieldConfig.defaults.custom.withLineWidth(1)
                      .fieldConfig.defaults.custom.withFillOpacity(10)
                      .fieldConfig.defaults.custom.withLineInterpolation('linear')
                      .fieldConfig.defaults.custom.withShowPoints('never')
                      .fieldConfig.defaults.custom.withSpanNulls(false)
                      .addThresholds([
    { color: 'green', value: null },
    { color: 'red', value: 80 },
  ])
                      .options.legend.withDisplayMode('list')
                      .options.legend.withPlacement('bottom')
                      .options.tooltip.withMode('single')
                      .options.tooltip.withSort('none'),
}
