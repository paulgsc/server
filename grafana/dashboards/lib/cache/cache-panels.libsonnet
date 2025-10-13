{
  // === Cache Operations Overview ===
  cacheOperationsTotal: {
    title: 'Cache Operations Total',
    type: 'stat',
    targets: [
      {
        expr: 'sum(rate(cache_operations_total[5m]))',
        legendFormat: 'Total Operations/sec',
      },
    ],
    fieldConfig: {
      defaults: {
        unit: 'reqps',
        color: { mode: 'thresholds' },
        thresholds: {
          steps: [
            { color: 'green', value: null },
            { color: 'yellow', value: 100 },
            { color: 'red', value: 1000 },
          ],
        },
      },
    },
  },

  cacheOperationsByType: {
    title: 'Cache Operations by Type',
    type: 'timeseries',
    targets: [
      {
        expr: 'sum(rate(cache_operations_total[5m])) by (operation)',
        legendFormat: '{{operation}}',
      },
    ],
    fieldConfig: {
      defaults: {
        unit: 'reqps',
        custom: {
          drawStyle: 'line',
          lineInterpolation: 'linear',
          lineWidth: 2,
          fillOpacity: 10,
          gradientMode: 'none',
          spanNulls: false,
          pointSize: 5,
          stacking: { mode: 'none', group: 'A' },
        },
      },
    },
    options: {
      legend: { displayMode: 'table', placement: 'right', calcs: ['last', 'max', 'mean'] },
      tooltip: { mode: 'multi', sort: 'none' },
    },
  },

  cacheSuccessRate: {
    title: 'Cache Operation Success Rate',
    type: 'stat',
    targets: [
      {
        expr: 'sum(rate(cache_operations_total{result="success"}[5m])) / sum(rate(cache_operations_total[5m])) * 100',
        legendFormat: 'Success Rate',
      },
    ],
    fieldConfig: {
      defaults: {
        unit: 'percent',
        min: 0,
        max: 100,
        color: { mode: 'thresholds' },
        thresholds: {
          steps: [
            { color: 'red', value: null },
            { color: 'yellow', value: 95 },
            { color: 'green', value: 99 },
          ],
        },
      },
    },
  },

  // === Cache Hit/Miss Metrics ===
  cacheHitRate: {
    title: 'Cache Hit Rate',
    type: 'stat',
    targets: [
      {
        expr: 'sum(rate(cache_hits_total[5m])) / (sum(rate(cache_hits_total[5m])) + sum(rate(cache_misses_total[5m]))) * 100',
        legendFormat: 'Hit Rate',
      },
    ],
    fieldConfig: {
      defaults: {
        unit: 'percent',
        min: 0,
        max: 100,
        color: { mode: 'thresholds' },
        thresholds: {
          steps: [
            { color: 'red', value: null },
            { color: 'yellow', value: 70 },
            { color: 'green', value: 90 },
          ],
        },
      },
    },
  },

  cacheHitsAndMisses: {
    title: 'Cache Hits vs Misses',
    type: 'timeseries',
    targets: [
      {
        expr: 'sum(rate(cache_hits_total[5m]))',
        legendFormat: 'Hits/sec',
      },
      {
        expr: 'sum(rate(cache_misses_total[5m]))',
        legendFormat: 'Misses/sec',
      },
    ],
    fieldConfig: {
      defaults: {
        unit: 'reqps',
        custom: {
          drawStyle: 'line',
          lineInterpolation: 'linear',
          lineWidth: 2,
          fillOpacity: 20,
          gradientMode: 'opacity',
          spanNulls: false,
          pointSize: 5,
          stacking: { mode: 'none', group: 'A' },
        },
      },
      overrides: [
        {
          matcher: { id: 'byName', options: 'Hits/sec' },
          properties: [{ id: 'color', value: { mode: 'fixed', fixedColor: 'green' } }],
        },
        {
          matcher: { id: 'byName', options: 'Misses/sec' },
          properties: [{ id: 'color', value: { mode: 'fixed', fixedColor: 'red' } }],
        },
      ],
    },
    options: {
      legend: { displayMode: 'table', placement: 'bottom', calcs: ['last', 'max', 'mean'] },
      tooltip: { mode: 'multi', sort: 'none' },
    },
  },

  cacheHitsByOperation: {
    title: 'Cache Hits by Operation Type',
    type: 'timeseries',
    targets: [
      {
        expr: 'sum(rate(cache_hits_total[5m])) by (operation)',
        legendFormat: '{{operation}}',
      },
    ],
    fieldConfig: {
      defaults: {
        unit: 'reqps',
        custom: {
          drawStyle: 'line',
          lineInterpolation: 'linear',
          lineWidth: 2,
          fillOpacity: 15,
          gradientMode: 'none',
          spanNulls: false,
          pointSize: 5,
          stacking: { mode: 'normal', group: 'A' },
        },
      },
    },
    options: {
      legend: { displayMode: 'table', placement: 'right', calcs: ['last', 'max'] },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
  },

  // === Performance Metrics ===
  cacheOperationDuration: {
    title: 'Cache Operation Duration',
    type: 'timeseries',
    targets: [
      {
        expr: 'histogram_quantile(0.50, sum(rate(cache_operation_duration_seconds_bucket[5m])) by (operation, le))',
        legendFormat: '{{operation}} p50',
      },
      {
        expr: 'histogram_quantile(0.95, sum(rate(cache_operation_duration_seconds_bucket[5m])) by (operation, le))',
        legendFormat: '{{operation}} p95',
      },
      {
        expr: 'histogram_quantile(0.99, sum(rate(cache_operation_duration_seconds_bucket[5m])) by (operation, le))',
        legendFormat: '{{operation}} p99',
      },
    ],
    fieldConfig: {
      defaults: {
        unit: 's',
        custom: {
          drawStyle: 'line',
          lineInterpolation: 'linear',
          lineWidth: 2,
          fillOpacity: 10,
          gradientMode: 'none',
          spanNulls: false,
          pointSize: 5,
          stacking: { mode: 'none', group: 'A' },
        },
      },
    },
    options: {
      legend: { displayMode: 'table', placement: 'right', calcs: ['last', 'max'] },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
  },

  redisConnectionDuration: {
    title: 'Redis Connection Duration',
    type: 'timeseries',
    targets: [
      {
        expr: 'histogram_quantile(0.50, sum(rate(cache_redis_connection_duration_seconds_bucket[5m])) by (le))',
        legendFormat: 'Connection p50',
      },
      {
        expr: 'histogram_quantile(0.95, sum(rate(cache_redis_connection_duration_seconds_bucket[5m])) by (le))',
        legendFormat: 'Connection p95',
      },
      {
        expr: 'histogram_quantile(0.99, sum(rate(cache_redis_connection_duration_seconds_bucket[5m])) by (le))',
        legendFormat: 'Connection p99',
      },
    ],
    fieldConfig: {
      defaults: {
        unit: 's',
        custom: {
          drawStyle: 'line',
          lineInterpolation: 'linear',
          lineWidth: 2,
          fillOpacity: 10,
          gradientMode: 'none',
          spanNulls: false,
          pointSize: 5,
          stacking: { mode: 'none', group: 'A' },
        },
      },
    },
    options: {
      legend: { displayMode: 'table', placement: 'bottom', calcs: ['last', 'max'] },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
  },

  // === Compression Metrics ===
  compressionOperations: {
    title: 'Compression Operations',
    type: 'stat',
    targets: [
      {
        expr: 'sum(rate(cache_compressions_total[5m]))',
        legendFormat: 'Compressions/sec',
      },
    ],
    fieldConfig: {
      defaults: {
        unit: 'reqps',
        color: { mode: 'thresholds' },
        thresholds: {
          steps: [
            { color: 'green', value: null },
            { color: 'yellow', value: 50 },
            { color: 'red', value: 200 },
          ],
        },
      },
    },
  },

  compressionRatio: {
    title: 'Compression Ratio',
    type: 'gauge',
    targets: [
      {
        expr: 'cache_compression_ratio',
        legendFormat: 'Ratio',
      },
    ],
    fieldConfig: {
      defaults: {
        unit: 'short',
        min: 1,
        max: 10,
        color: { mode: 'thresholds' },
        thresholds: {
          steps: [
            { color: 'red', value: null },
            { color: 'yellow', value: 2 },
            { color: 'green', value: 4 },
          ],
        },
      },
    },
    options: {
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
  },

  compressionDuration: {
    title: 'Compression Duration',
    type: 'timeseries',
    targets: [
      {
        expr: 'histogram_quantile(0.50, sum(rate(cache_compression_duration_seconds_bucket[5m])) by (le))',
        legendFormat: 'Compression p50',
      },
      {
        expr: 'histogram_quantile(0.95, sum(rate(cache_compression_duration_seconds_bucket[5m])) by (le))',
        legendFormat: 'Compression p95',
      },
      {
        expr: 'histogram_quantile(0.50, sum(rate(cache_decompression_duration_seconds_bucket[5m])) by (le))',
        legendFormat: 'Decompression p50',
      },
      {
        expr: 'histogram_quantile(0.95, sum(rate(cache_decompression_duration_seconds_bucket[5m])) by (le))',
        legendFormat: 'Decompression p95',
      },
    ],
    fieldConfig: {
      defaults: {
        unit: 's',
        custom: {
          drawStyle: 'line',
          lineInterpolation: 'linear',
          lineWidth: 2,
          fillOpacity: 10,
          gradientMode: 'none',
          spanNulls: false,
          pointSize: 5,
          stacking: { mode: 'none', group: 'A' },
        },
      },
    },
    options: {
      legend: { displayMode: 'table', placement: 'right', calcs: ['last', 'max'] },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
  },

  // === Data Size Metrics ===
  cacheDataSize: {
    title: 'Cache Data Size Distribution',
    type: 'timeseries',
    targets: [
      {
        expr: 'histogram_quantile(0.50, sum(rate(cache_data_size_bytes_bucket[5m])) by (le))',
        legendFormat: 'Original Size p50',
      },
      {
        expr: 'histogram_quantile(0.95, sum(rate(cache_data_size_bytes_bucket[5m])) by (le))',
        legendFormat: 'Original Size p95',
      },
      {
        expr: 'histogram_quantile(0.50, sum(rate(cache_compressed_size_bytes_bucket[5m])) by (le))',
        legendFormat: 'Compressed Size p50',
      },
      {
        expr: 'histogram_quantile(0.95, sum(rate(cache_compressed_size_bytes_bucket[5m])) by (le))',
        legendFormat: 'Compressed Size p95',
      },
    ],
    fieldConfig: {
      defaults: {
        unit: 'bytes',
        custom: {
          drawStyle: 'line',
          lineInterpolation: 'linear',
          lineWidth: 2,
          fillOpacity: 15,
          gradientMode: 'none',
          spanNulls: false,
          pointSize: 5,
          stacking: { mode: 'none', group: 'A' },
        },
      },
    },
    options: {
      legend: { displayMode: 'table', placement: 'right', calcs: ['last', 'max'] },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
  },

  dataSizeComparison: {
    title: 'Data Size Comparison (Original vs Compressed)',
    type: 'bargauge',
    targets: [
      {
        expr: 'sum(rate(cache_data_size_bytes_sum[5m])) / sum(rate(cache_data_size_bytes_count[5m]))',
        legendFormat: 'Avg Original Size',
      },
      {
        expr: 'sum(rate(cache_compressed_size_bytes_sum[5m])) / sum(rate(cache_compressed_size_bytes_count[5m]))',
        legendFormat: 'Avg Compressed Size',
      },
    ],
    fieldConfig: {
      defaults: {
        unit: 'bytes',
        color: { mode: 'palette-classic' },
        custom: {
          displayMode: 'basic',
          orientation: 'horizontal',
        },
      },
    },
    options: {
      reduceOptions: {
        values: false,
        calcs: ['lastNotNull'],
        fields: '',
      },
      orientation: 'horizontal',
      displayMode: 'basic',
    },
  },

  // === Error and Retry Metrics ===
  cacheErrors: {
    title: 'Cache Errors by Type',
    type: 'timeseries',
    targets: [
      {
        expr: 'sum(rate(cache_errors_total[5m])) by (error_type)',
        legendFormat: '{{error_type}}',
      },
    ],
    fieldConfig: {
      defaults: {
        unit: 'reqps',
        custom: {
          drawStyle: 'line',
          lineInterpolation: 'linear',
          lineWidth: 2,
          fillOpacity: 20,
          gradientMode: 'opacity',
          spanNulls: false,
          pointSize: 5,
          stacking: { mode: 'normal', group: 'A' },
        },
      },
    },
    options: {
      legend: { displayMode: 'table', placement: 'right', calcs: ['last', 'max', 'total'] },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
  },

  cacheRetries: {
    title: 'Cache Operation Retries',
    type: 'timeseries',
    targets: [
      {
        expr: 'sum(rate(cache_retries_total[5m])) by (operation)',
        legendFormat: '{{operation}}',
      },
    ],
    fieldConfig: {
      defaults: {
        unit: 'reqps',
        custom: {
          drawStyle: 'line',
          lineInterpolation: 'linear',
          lineWidth: 2,
          fillOpacity: 15,
          gradientMode: 'none',
          spanNulls: false,
          pointSize: 5,
          stacking: { mode: 'normal', group: 'A' },
        },
      },
    },
    options: {
      legend: { displayMode: 'table', placement: 'right', calcs: ['last', 'max', 'total'] },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
  },

  errorRate: {
    title: 'Cache Error Rate',
    type: 'stat',
    targets: [
      {
        expr: 'sum(rate(cache_operations_total{result="error"}[5m])) / sum(rate(cache_operations_total[5m])) * 100',
        legendFormat: 'Error Rate',
      },
    ],
    fieldConfig: {
      defaults: {
        unit: 'percent',
        min: 0,
        max: 100,
        color: { mode: 'thresholds' },
        thresholds: {
          steps: [
            { color: 'green', value: null },
            { color: 'yellow', value: 1 },
            { color: 'red', value: 5 },
          ],
        },
      },
    },
  },

  // === TTL and Access Pattern Metrics ===
  cacheTTL: {
    title: 'Cache TTL Distribution',
    type: 'timeseries',
    targets: [
      {
        expr: 'histogram_quantile(0.50, sum(rate(cache_ttl_seconds_bucket[5m])) by (le))',
        legendFormat: 'TTL p50',
      },
      {
        expr: 'histogram_quantile(0.95, sum(rate(cache_ttl_seconds_bucket[5m])) by (le))',
        legendFormat: 'TTL p95',
      },
      {
        expr: 'histogram_quantile(0.99, sum(rate(cache_ttl_seconds_bucket[5m])) by (le))',
        legendFormat: 'TTL p99',
      },
    ],
    fieldConfig: {
      defaults: {
        unit: 's',
        custom: {
          drawStyle: 'line',
          lineInterpolation: 'linear',
          lineWidth: 2,
          fillOpacity: 10,
          gradientMode: 'none',
          spanNulls: false,
          pointSize: 5,
          stacking: { mode: 'none', group: 'A' },
        },
      },
    },
    options: {
      legend: { displayMode: 'table', placement: 'bottom', calcs: ['last', 'max'] },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
  },

  cacheAccessCount: {
    title: 'Cache Access Count Distribution',
    type: 'timeseries',
    targets: [
      {
        expr: 'histogram_quantile(0.50, sum(rate(cache_access_count_bucket[5m])) by (le))',
        legendFormat: 'Access Count p50',
      },
      {
        expr: 'histogram_quantile(0.95, sum(rate(cache_access_count_bucket[5m])) by (le))',
        legendFormat: 'Access Count p95',
      },
    ],
    fieldConfig: {
      defaults: {
        unit: 'short',
        custom: {
          drawStyle: 'line',
          lineInterpolation: 'linear',
          lineWidth: 2,
          fillOpacity: 15,
          gradientMode: 'none',
          spanNulls: false,
          pointSize: 5,
          stacking: { mode: 'none', group: 'A' },
        },
      },
    },
    options: {
      legend: { displayMode: 'table', placement: 'bottom', calcs: ['last', 'max'] },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
  },

  cacheEntryAge: {
    title: 'Cache Entry Age',
    type: 'gauge',
    targets: [
      {
        expr: 'cache_entry_age_seconds',
        legendFormat: 'Entry Age',
      },
    ],
    fieldConfig: {
      defaults: {
        unit: 's',
        min: 0,
        color: { mode: 'thresholds' },
        thresholds: {
          steps: [
            { color: 'green', value: null },
            { color: 'yellow', value: 3600 },
            { color: 'red', value: 86400 },
          ],
        },
      },
    },
    options: {
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
  },

  // === Summary Panels ===
  cacheHealthStatus: {
    title: 'Cache Health Status',
    type: 'table',
    targets: [
      {
        expr: 'sum(rate(cache_operations_total[5m]))',
        legendFormat: 'Operations/sec',
        format: 'table',
        instant: true,
      },
      {
        expr: 'sum(rate(cache_hits_total[5m])) / (sum(rate(cache_hits_total[5m])) + sum(rate(cache_misses_total[5m]))) * 100',
        legendFormat: 'Hit Rate (%)',
        format: 'table',
        instant: true,
      },
      {
        expr: 'sum(rate(cache_operations_total{result="error"}[5m])) / sum(rate(cache_operations_total[5m])) * 100',
        legendFormat: 'Error Rate (%)',
        format: 'table',
        instant: true,
      },
      {
        expr: 'cache_compression_ratio',
        legendFormat: 'Compression Ratio',
        format: 'table',
        instant: true,
      },
    ],
    fieldConfig: {
      defaults: {
        custom: {
          align: 'auto',
          displayMode: 'auto',
        },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            { color: 'green', value: null },
            { color: 'red', value: 80 },
          ],
        },
      },
      overrides: [
        {
          matcher: { id: 'byName', options: 'Hit Rate (%)' },
          properties: [
            {
              id: 'custom.displayMode',
              value: 'color-background',
            },
            {
              id: 'thresholds',
              value: {
                steps: [
                  { color: 'red', value: null },
                  { color: 'yellow', value: 70 },
                  { color: 'green', value: 90 },
                ],
              },
            },
          ],
        },
        {
          matcher: { id: 'byName', options: 'Error Rate (%)' },
          properties: [
            {
              id: 'custom.displayMode',
              value: 'color-background',
            },
            {
              id: 'thresholds',
              value: {
                steps: [
                  { color: 'green', value: null },
                  { color: 'yellow', value: 1 },
                  { color: 'red', value: 5 },
                ],
              },
            },
          ],
        },
      ],
    },
    options: {
      showHeader: true,
    },
    transformations: [
      {
        id: 'merge',
        options: {},
      },
      {
        id: 'organize',
        options: {
          excludeByName: {
            Time: true,
          },
          indexByName: {},
          renameByName: {},
        },
      },
    ],
  },
}
