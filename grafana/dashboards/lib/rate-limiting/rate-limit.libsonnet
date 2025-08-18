{
  // Panel: Total Request Rate
  totalRequests: {
    datasource: 'Prometheus',
    fieldConfig: {
      defaults: {
        custom: {},
        unit: 'reqps',
      },
      overrides: [],
    },
    gridPos: { h: 8, w: 12, x: 0, y: 0 },
    id: 1,
    options: { legend: {}, tooltip: {} },
    targets: [
      {
        datasource: 'Prometheus',
        expr: 'sum(rate(http_requests_total[1m]))',
        legendFormat: 'Total RPS',
        refId: 'A',
      },
    ],
    title: 'Total Request Rate (RPS)',
    type: 'timeseries',
  },

  // Panel: Status Code Breakdown
  statusCodes: {
    datasource: 'Prometheus',
    fieldConfig: {
      defaults: {
        custom: {},
        unit: 'reqps',
      },
      overrides: [],
    },
    gridPos: { h: 8, w: 12, x: 12, y: 0 },
    id: 2,
    options: { legend: {}, tooltip: {}, stacking: { mode: 'normal' } },
    targets: [
      {
        datasource: 'Prometheus',
        expr: 'sum by (status) (rate(http_requests_total[1m]))',
        legendFormat: 'HTTP {{status}}',
        refId: 'A',
      },
    ],
    title: 'HTTP Status Codes',
    type: 'timeseries',
  },

  // Panel: 429 Throttled Requests
  throttled: {
    datasource: 'Prometheus',
    fieldConfig: {
      defaults: {
        custom: {},
        unit: 'reqps',
      },
      overrides: [],
    },
    gridPos: { h: 8, w: 12, x: 0, y: 8 },
    id: 3,
    options: { legend: {}, tooltip: {} },
    targets: [
      {
        datasource: 'Prometheus',
        expr: 'sum(rate(http_requests_total{status="429"}[1m]))',
        legendFormat: '429 responses',
        refId: 'A',
      },
    ],
    title: '429 Too Many Requests',
    type: 'timeseries',
  },

  // Panel: Request Rate by Route
  byRoute: {
    datasource: 'Prometheus',
    fieldConfig: {
      defaults: {
        custom: {},
        unit: 'reqps',
      },
      overrides: [],
    },
    gridPos: { h: 8, w: 12, x: 12, y: 8 },
    id: 4,
    options: { legend: {}, tooltip: {} },
    targets: [
      {
        datasource: 'Prometheus',
        expr: 'sum by (route) (rate(http_requests_total[1m]))',
        legendFormat: '{{route}}',
        refId: 'A',
      },
    ],
    title: 'Request Rate by Endpoint',
    type: 'timeseries',
  },

  // Panel: Top Clients by Request Volume
  topClients: {
    datasource: 'Prometheus',
    fieldConfig: {
      defaults: {
        custom: {
          align: 'auto',
          displayMode: 'auto',
        },
      },
      overrides: [],
    },
    gridPos: { h: 8, w: 24, x: 0, y: 16 },
    id: 5,
    options: {
      frameIndex: 0,
      showHeader: true,
      sortBy: [{ desc: true, displayName: 'Value #A' }],
    },
    pluginVersion: '8.5.3',
    targets: [
      {
        datasource: 'Prometheus',
        expr: 'topk(10, sum by (client_ip) (rate(http_requests_total_by_client[5m])))',
        format: 'table',
        instant: true,
        refId: 'A',
      },
    ],
    title: 'Top Clients (Last 5m)',
    transformations: [
      {
        id: 'seriesToRows',
        options: {},
      },
    ],
    type: 'table',
  },

  // Panel: Invariant Violation
  invariant: {
    datasource: 'Prometheus',
    fieldConfig: {
      defaults: {
        mappings: [
          {
            op: '=',
            text: 'Firing',
            type: 1,
            value: 1,
          },
          {
            op: '=',
            text: 'Healthy',
            type: 1,
            value: 0,
          },
        ],
        thresholds: {
          mode: 'absolute',
          steps: [
            { color: 'green', value: null },
            { color: 'red', value: 1 },
          ],
        },
        unit: 'none',
      },
      overrides: [],
    },
    gridPos: { h: 8, w: 24, x: 0, y: 24 },
    id: 6,
    options: {
      colorMode: 'value',
      graphMode: 'none',
      justifyMode: 'auto',
      orientation: 'auto',
      textMode: 'auto',
    },
    pluginVersion: '8.5.3',
    targets: [
      {
        datasource: 'Prometheus',
        expr: '(\n  sum(rate(http_requests_total[1m])) > 20\n)\nand\n(\n  sum(rate(http_requests_total{status="429"}[1m])) == 0\n)',
        instant: true,
        refId: 'A',
      },
    ],
    title: '🚨 Invariant Violation: High Load + No Throttling',
    type: 'stat',
  },
}
