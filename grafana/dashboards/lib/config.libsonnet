{
  // Prometheus datasource configuration
  prometheusDataSource: {
    type: 'prometheus',
    uid: 'PB0E20699',
  },

  // Dashboard settings
  dashboardSettings: {
    refresh: '5s',
    schemaVersion: 38,
    tags: ['rust', 'axum', 'prometheus'],
    timeRange: {
      from: 'now-15m',
      to: 'now',
    },
  },

  // Panel ID counter (helps avoid ID conflicts)
  panelIds: {
    httpRequestRate: 2,
    httpLatency: 3,
    operationDuration: 4,
    cacheHitsMisses: 5,
    totalRequests: 6,
    totalCacheOps: 7,
    cacheHitRate: 8,
    rateLimitedRequests: 9,
  },
}
