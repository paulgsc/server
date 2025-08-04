
local grafana = import 'grafonnet/grafana.libsonnet';
local timeSeries = grafana.timeSeries;
local table = grafana.table;
local prometheus = grafana.prometheus;

{
    // Cache Hit Rate Time Series Panel
    hitRate(
            title='Cache Hit Rate %',
            datasource='${DS_PROMETHEUS}',
            handlerFilter='$handler'
           ):: timeSeries.new(
               title=title,
               datasource=datasource,
               )
           .addTarget(
                   prometheus.new(
                       expr=|||
                       (
                        rate(cache_operations_total{handler=~"%(filter)s", result="hit"}[5m]) /
                        (
                         rate(cache_operations_total{handler=~"%(filter)s", result="hit"}[5m]) +
                         rate(cache_operations_total{handler=~"%(filter)s", result="miss"}[5m])
                        )
                       ) * 100
                       ||| % { filter: handlerFilter },
                       legendFormat='{{handler}} Cache Hit Rate',
                       )
                   )
           .standardOptions.withUnit('percent')
           .standardOptions.withMax(100)
           .standardOptions.withMin(0)
           .fieldConfig.defaults.custom.withDrawStyle('line')
           .fieldConfig.defaults.custom.withLineWidth(2)
           .fieldConfig.defaults.custom.withFillOpacity(25)
           .fieldConfig.defaults.custom.withLineInterpolation('smooth')
           .fieldConfig.defaults.custom.withShowPoints('never')
           .fieldConfig.defaults.custom.withSpanNulls(true)
           .fieldConfig.defaults.custom.withGradientMode('opacity')
           .addThresholds([
                   { color: 'red', value: null },
                   { color: 'yellow', value: 50 },
                   { color: 'green', value: 80 },
           ])
           .options.legend.withDisplayMode('table')
           .options.legend.withPlacement('right')
           .options.legend.withCalcs(['last', 'mean'])
           .options.tooltip.withMode('multi')
           .options.tooltip.withSort('desc'),

        // Cache Operations Rate Panel
        operationsRate(
                title='Cache Operations Rate',
                datasource='${DS_PROMETHEUS}',
                handlerFilter='$handler'
                ):: timeSeries.new(
                    title=title,
                    datasource=datasource,
                    )
                .addTargets([
                        prometheus.new(
                            expr='rate(cache_operations_total{handler=~"%(filter)s", result="hit"}[5m])' % { filter: handlerFilter },
                            legendFormat='{{handler}} - Cache Hits',
                            ),
                        prometheus.new(
                            expr='rate(cache_operations_total{handler=~"%(filter)s", result="miss"}[5m])' % { filter: handlerFilter },
                            legendFormat='{{handler}} - Cache Misses',
                            ),
                        prometheus.new(
                            expr='rate(cache_operations_total{handler=~"%(filter)s", result="error"}[5m])' % { filter: handlerFilter },
                            legendFormat='{{handler}} - Cache Errors',
                            ),
                ])
                .standardOptions.withUnit('ops')
                .fieldConfig.defaults.custom.withDrawStyle('bars')
                .fieldConfig.defaults.custom.withFillOpacity(80)
                .fieldConfig.defaults.custom.withLineWidth(1)
                .fieldConfig.defaults.custom.withStacking({ group: 'A', mode: 'normal' })
                .options.legend.withDisplayMode('list')
                .options.legend.withPlacement('bottom')
                .options.tooltip.withMode('multi')
                .options.tooltip.withSort('desc'),

        // Cache Performance Summary Table
        summary(
                title='Cache Performance Summary',
                datasource='${DS_PROMETHEUS}',
                handlerFilter='$handler'
               ):: table.new(
                   title=title,
                   datasource=datasource,
                   )
               .addTarget(
                       prometheus.new(
                           expr=|||
                           (
                            sum by (handler) (rate(cache_operations_total{handler=~"%(filter)s", result="hit"}[5m])) /
                            sum by (handler) (
                                rate(cache_operations_total{handler=~"%(filter)s", result="hit"}[5m]) +
                                rate(cache_operations_total{handler=~"%(filter)s", result="miss"}[5m])
                                )
                           ) * 100
                           ||| % { filter: handlerFilter },
                           format='table',
                           instant=true,
                           )
                       )
               .addTransformations([
                       {
id: 'organize',
options: {
excludeByName: {
Time: true,
__name__: true,  // Fixed: was **name**
job: true,
instance: true,
},
renameByName: {
Value: 'Hit Rate %',
handler: 'Handler',
},
},
},
               ]),

               // Cache Error Rate Panel
               errorRate(
                       title='Cache Error Rate %',
                       datasource='${DS_PROMETHEUS}',
                       handlerFilter='$handler'
                       ):: timeSeries.new(
                           title=title,
                           datasource=datasource,
                           )
    .addTarget(
            prometheus.new(
                expr=|||
                (
                 sum by (handler) (rate(cache_operations_total{handler=~"%(filter)s", result="error"}[5m])) /
                 sum by (handler) (rate(cache_operations_total{handler=~"%(filter)s"}[5m]))
                ) * 100
                ||| % { filter: handlerFilter },
                legendFormat='{{handler}} Error Rate',
                )
            )
    .standardOptions.withUnit('percent')
.standardOptions.withMin(0)
    .fieldConfig.defaults.custom.withDrawStyle('line')
    .fieldConfig.defaults.custom.withLineWidth(2)
.fieldConfig.defaults.custom.withFillOpacity(10)
    .addThresholds([
            { color: 'green', value: null },
            { color: 'yellow', value: 1 },
            { color: 'red', value: 5 },
    ]),

    // Cache Size/Usage Panel
    size(
            title='Cache Size',
            datasource='${DS_PROMETHEUS}',
            handlerFilter='$handler'
        ):: timeSeries.new(
            title=title,
            datasource=datasource,
            )
    .addTarget(
            prometheus.new(
                expr='cache_size_bytes{handler=~"%(filter)s"}' % { filter: handlerFilter },
                legendFormat='{{handler}} Cache Size',
                )
            )
    .standardOptions.withUnit('bytes')
    .fieldConfig.defaults.custom.withDrawStyle('line')
    .fieldConfig.defaults.custom.withLineWidth(2),
    }
