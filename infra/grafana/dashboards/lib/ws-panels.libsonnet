{
  // === Connection Lifecycle Panels ===

  wsConnectionsActive: {
    title: 'Active WebSocket Connections',
    type: 'stat',
    targets: [
      {
        expr: 'sum(ws_connection_states{state="active"})',
        legendFormat: 'Active',
        refId: 'A',
      },
      {
        expr: 'sum(ws_connection_states{state="stale"})',
        legendFormat: 'Stale',
        refId: 'B',
      },
    ],
    fieldConfig: {
      defaults: {
        color: { mode: 'thresholds' },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            { value: null, color: 'green' },
            { value: 100, color: 'orange' },
            { value: 500, color: 'red' },
          ],
        },
        unit: 'short',
      },
    },
    options: {
      colorMode: 'value',
      graphMode: 'area',
      justifyMode: 'auto',
      orientation: 'auto',
      textMode: 'auto',
      reduceOptions: { calcs: ['lastNotNull'], fields: '', values: false },
    },
  },

  wsStaleConnections: {
    title: 'Stale Connections',
    type: 'stat',
    targets: [
      {
        expr: 'sum(ws_connection_states{state="stale"})',
        legendFormat: 'Stale',
        refId: 'A',
      },
    ],
    fieldConfig: {
      defaults: {
        color: { mode: 'thresholds' },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            { value: null, color: 'green' },
            { value: 5, color: 'orange' },
            { value: 20, color: 'red' },
          ],
        },
        unit: 'short',
      },
    },
    options: {
      colorMode: 'value',
      graphMode: 'none',
      textMode: 'auto',
      reduceOptions: { calcs: ['lastNotNull'], fields: '', values: false },
    },
  },

  wsConnectionStateDistribution: {
    title: 'Connection State Distribution',
    type: 'timeseries',
    targets: [
      { expr: 'ws_connection_states{state="active"}', legendFormat: 'Active', refId: 'A' },
      { expr: 'ws_connection_states{state="stale"}', legendFormat: 'Stale', refId: 'B' },
    ],
    fieldConfig: {
      defaults: {
        color: { mode: 'palette-classic' },
        custom: {
          drawStyle: 'line',
          lineInterpolation: 'linear',
          lineWidth: 1,
          fillOpacity: 10,
          showPoints: 'never',
          pointSize: 5,
          barAlignment: 0,
          axisPlacement: 'auto',
          axisLabel: '',
          scaleDistribution: { type: 'linear' },
          hideFrom: { legend: false, tooltip: false, vis: false },
          stacking: { group: 'A', mode: 'none' },
          thresholdsStyle: { mode: 'off' },
        },
        unit: 'short',
      },
    },
    options: {
      legend: { displayMode: 'list', placement: 'bottom', calcs: [] },
      tooltip: { mode: 'multi', sort: 'none' },
    },
  },

  wsConnectionRate: {
    title: 'Connection Rate (Created/Removed)',
    type: 'timeseries',
    targets: [
      { expr: 'rate(ws_connection_lifecycle_total{event="created"}[5m])', legendFormat: 'Created/sec', refId: 'A' },
      { expr: 'rate(ws_connection_lifecycle_total{event="removed"}[5m])', legendFormat: 'Removed/sec', refId: 'B' },
    ],
    fieldConfig: {
      defaults: {
        color: { mode: 'palette-classic' },
        custom: {
          drawStyle: 'line',
          lineInterpolation: 'linear',
          lineWidth: 1,
          fillOpacity: 10,
          showPoints: 'never',
          pointSize: 5,
          barAlignment: 0,
          axisPlacement: 'auto',
          axisLabel: '',
          scaleDistribution: { type: 'linear' },
          hideFrom: { legend: false, tooltip: false, vis: false },
          stacking: { group: 'A', mode: 'none' },
          thresholdsStyle: { mode: 'off' },
        },
        unit: 'ops',
      },
    },
    options: {
      legend: { displayMode: 'list', placement: 'bottom', calcs: [] },
      tooltip: { mode: 'multi', sort: 'none' },
    },
  },

  wsConnectionDuration: {
    title: 'Connection Duration (p95)',
    type: 'timeseries',
    targets: [
      {
        expr: 'histogram_quantile(0.95, sum(rate(ws_connection_duration_seconds_bucket[5m])) by (le, end_reason))',
        legendFormat: '{{end_reason}}',
        refId: 'A',
      },
    ],
    fieldConfig: {
      defaults: {
        color: { mode: 'palette-classic' },
        custom: {
          drawStyle: 'line',
          lineInterpolation: 'linear',
          lineWidth: 1,
          fillOpacity: 10,
          showPoints: 'never',
          pointSize: 5,
          barAlignment: 0,
          axisPlacement: 'auto',
          axisLabel: '',
          scaleDistribution: { type: 'linear' },
          hideFrom: { legend: false, tooltip: false, vis: false },
          stacking: { group: 'A', mode: 'none' },
          thresholdsStyle: { mode: 'off' },
        },
        unit: 's',
      },
    },
    options: {
      legend: { displayMode: 'list', placement: 'bottom', calcs: [] },
      tooltip: { mode: 'multi', sort: 'none' },
    },
  },

  avgConnectionDuration: {
    title: 'Avg Connection Duration',
    type: 'stat',
    targets: [
      {
        expr: 'avg(rate(ws_connection_duration_seconds_sum[5m]) / rate(ws_connection_duration_seconds_count[5m]))',
        legendFormat: 'Average',
        refId: 'A',
      },
    ],
    fieldConfig: {
      defaults: {
        color: { mode: 'thresholds' },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            { value: null, color: 'green' },
            { value: 300, color: 'orange' },
            { value: 600, color: 'red' },
          ],
        },
        unit: 's',
      },
    },
    options: {
      colorMode: 'value',
      graphMode: 'area',
      justifyMode: 'auto',
      orientation: 'auto',
      textMode: 'auto',
      reduceOptions: { calcs: ['lastNotNull'], fields: '', values: false },
    },
  },

  // === Message Processing Panels ===

  wsMessageRate: {
    title: 'Message Processing Rate',
    type: 'timeseries',
    targets: [
      { expr: 'rate(ws_connection_messages_total[5m])', legendFormat: '{{message_type}}', refId: 'A' },
    ],
    fieldConfig: {
      defaults: {
        color: { mode: 'palette-classic' },
        custom: {
          drawStyle: 'line',
          lineInterpolation: 'linear',
          lineWidth: 1,
          fillOpacity: 10,
          showPoints: 'never',
          pointSize: 5,
          barAlignment: 0,
          axisPlacement: 'auto',
          axisLabel: '',
          scaleDistribution: { type: 'linear' },
          hideFrom: { legend: false, tooltip: false, vis: false },
          stacking: { group: 'A', mode: 'none' },
          thresholdsStyle: { mode: 'off' },
        },
        unit: 'msgps',
      },
    },
    options: {
      legend: { displayMode: 'list', placement: 'bottom', calcs: [] },
      tooltip: { mode: 'multi', sort: 'none' },
    },
  },

  wsMessageTypeDistribution: {
    title: 'Message Type Distribution',
    type: 'timeseries',
    targets: [
      { expr: 'rate(ws_connection_messages_total[5m])', legendFormat: '{{message_type}}', refId: 'A' },
    ],
    fieldConfig: {
      defaults: {
        color: { mode: 'palette-classic' },
        custom: {
          drawStyle: 'line',
          lineInterpolation: 'linear',
          lineWidth: 1,
          fillOpacity: 10,
          showPoints: 'never',
          pointSize: 5,
          barAlignment: 0,
          axisPlacement: 'auto',
          axisLabel: '',
          scaleDistribution: { type: 'linear' },
          hideFrom: { legend: false, tooltip: false, vis: false },
          stacking: { group: 'A', mode: 'normal' },
          thresholdsStyle: { mode: 'off' },
        },
        unit: 'msgps',
      },
    },
    options: {
      legend: { displayMode: 'list', placement: 'bottom', calcs: [] },
      tooltip: { mode: 'multi', sort: 'none' },
    },
  },

  // === Subscription Panels ===

  wsActiveSubscriptions: {
    title: 'Active Subscriptions by Event Type',
    type: 'timeseries',
    targets: [
      { expr: 'ws_connection_subscriptions', legendFormat: '{{event_type}}', refId: 'A' },
    ],
    fieldConfig: {
      defaults: {
        color: { mode: 'palette-classic' },
        custom: {
          drawStyle: 'line',
          lineInterpolation: 'linear',
          lineWidth: 1,
          fillOpacity: 10,
          showPoints: 'never',
          pointSize: 5,
          barAlignment: 0,
          axisPlacement: 'auto',
          axisLabel: '',
          scaleDistribution: { type: 'linear' },
          hideFrom: { legend: false, tooltip: false, vis: false },
          stacking: { group: 'A', mode: 'normal' },
          thresholdsStyle: { mode: 'off' },
        },
        unit: 'short',
      },
    },
    options: {
      legend: { displayMode: 'list', placement: 'bottom', calcs: [] },
      tooltip: { mode: 'multi', sort: 'none' },
    },
  },

  wsSubscriptionOperations: {
    title: 'Subscription Changes (Rate)',
    type: 'timeseries',
    targets: [
      { expr: 'rate(ws_connection_messages_total{message_type="subscribe"}[5m])', legendFormat: 'Subscribe', refId: 'A' },
      { expr: 'rate(ws_connection_messages_total{message_type="unsubscribe"}[5m])', legendFormat: 'Unsubscribe', refId: 'B' },
    ],
    fieldConfig: {
      defaults: {
        color: { mode: 'palette-classic' },
        custom: {
          drawStyle: 'line',
          lineInterpolation: 'linear',
          lineWidth: 1,
          fillOpacity: 10,
          showPoints: 'never',
          pointSize: 5,
          barAlignment: 0,
          axisPlacement: 'auto',
          axisLabel: '',
          scaleDistribution: { type: 'linear' },
          hideFrom: { legend: false, tooltip: false, vis: false },
          stacking: { group: 'A', mode: 'none' },
          thresholdsStyle: { mode: 'off' },
        },
        unit: 'ops',
      },
    },
    options: {
      legend: { displayMode: 'list', placement: 'bottom', calcs: [] },
      tooltip: { mode: 'multi', sort: 'none' },
    },
  },

  // === Client & System Monitoring ===

  wsClientConnections: {
    title: 'Client Type Distribution',
    type: 'timeseries',
    targets: [
      { expr: 'ws_client_connections', legendFormat: '{{client_type}}', refId: 'A' },
    ],
    fieldConfig: {
      defaults: {
        color: { mode: 'palette-classic' },
        custom: {
          drawStyle: 'line',
          lineInterpolation: 'linear',
          lineWidth: 1,
          fillOpacity: 10,
          showPoints: 'never',
          pointSize: 5,
          barAlignment: 0,
          axisPlacement: 'auto',
          axisLabel: '',
          scaleDistribution: { type: 'linear' },
          hideFrom: { legend: false, tooltip: false, vis: false },
          stacking: { group: 'A', mode: 'normal' },
          thresholdsStyle: { mode: 'off' },
        },
        unit: 'short',
      },
    },
    options: {
      legend: { displayMode: 'list', placement: 'bottom', calcs: [] },
      tooltip: { mode: 'multi', sort: 'none' },
    },
  },

  wsTimeoutMonitorOperations: {
    title: 'Timeout Monitor Operations',
    type: 'timeseries',
    targets: [
      { expr: 'rate(ws_timeout_monitor_operations_total{operation="mark_stale", result="success"}[5m])', legendFormat: 'Mark Stale', refId: 'A' },
      { expr: 'rate(ws_timeout_monitor_operations_total{operation="cleanup", result="success"}[5m])', legendFormat: 'Cleanup', refId: 'B' },
      { expr: 'rate(ws_timeout_monitor_operations_total{operation="health_check", result="success"}[5m])', legendFormat: 'Health Check', refId: 'C' },
    ],
    fieldConfig: {
      defaults: {
        color: { mode: 'palette-classic' },
        custom: {
          drawStyle: 'line',
          lineInterpolation: 'linear',
          lineWidth: 1,
          fillOpacity: 10,
          showPoints: 'never',
          pointSize: 5,
          barAlignment: 0,
          axisPlacement: 'auto',
          axisLabel: '',
          scaleDistribution: { type: 'linear' },
          hideFrom: { legend: false, tooltip: false, vis: false },
          stacking: { group: 'A', mode: 'none' },
          thresholdsStyle: { mode: 'off' },
        },
        unit: 'ops',
      },
    },
    options: {
      legend: { displayMode: 'list', placement: 'bottom', calcs: [] },
      tooltip: { mode: 'multi', sort: 'none' },
    },
  },

  // === Error Monitoring ===

  wsErrors: {
    title: 'Connection Error Rate',
    type: 'timeseries',
    targets: [
      { expr: 'rate(ws_connection_errors_total[5m])', legendFormat: '{{error_type}} - {{phase}}', refId: 'A' },
    ],
    fieldConfig: {
      defaults: {
        color: { mode: 'palette-classic' },
        custom: {
          drawStyle: 'line',
          lineInterpolation: 'linear',
          lineWidth: 1,
          fillOpacity: 10,
          showPoints: 'never',
          pointSize: 5,
          barAlignment: 0,
          axisPlacement: 'auto',
          axisLabel: '',
          scaleDistribution: { type: 'linear' },
          hideFrom: { legend: false, tooltip: false, vis: false },
          stacking: { group: 'A', mode: 'none' },
          thresholdsStyle: { mode: 'off' },
        },
        unit: 'eps',
      },
    },
    options: {
      legend: { displayMode: 'list', placement: 'bottom', calcs: [] },
      tooltip: { mode: 'multi', sort: 'none' },
    },
  },

  wsTopErrors: {
    title: 'Top Connection Errors (Last 1h)',
    type: 'table',
    targets: [
      {
        expr: 'topk(10, increase(ws_connection_errors_total[1h])) by (error_type, phase)',
        format: 'table',
        instant: true,
        legendFormat: '{{error_type}} - {{phase}}',
        refId: 'A',
      },
    ],
    fieldConfig: {
      defaults: {
        color: { mode: 'thresholds' },
        thresholds: {
          mode: 'absolute',
          steps: [
            { value: null, color: 'green' },
            { value: 5, color: 'red' },
          ],
        },
        custom: {
          align: 'auto',
          displayMode: 'auto',
          inspect: false,
        },
      },
      overrides: [],
    },
    options: {
      showHeader: true,
      displayMode: 'color-text',
    },
    transformations: [
      {
        id: 'organize',
        options: {
          excludeByName: { Time: true, __name__: true },
          renameByName: { Value: 'Count' },
        },
      },
    ],
  },
}
