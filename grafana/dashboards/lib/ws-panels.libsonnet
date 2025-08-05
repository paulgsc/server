local utils = import 'utils.libsonnet';

{
  // Connection Lifecycle Panels
  wsConnectionsActive: {
    title: 'Active WebSocket Connections',
    type: 'stat',
    targets: [
      {
        expr: 'sum(ws_connections_active{state="active"})',
        legendFormat: 'Active',
        refId: 'A',
      },
      {
        expr: 'sum(ws_connections_active{state="stale"})',
        legendFormat: 'Stale',
        refId: 'B',
      },
    ],
    fieldConfig: {
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
            {
              color: 'red',
              value: 1000,
            },
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
      reduceOptions: {
        calcs: ['lastNotNull'],
        fields: '',
        values: false,
      },
      textMode: 'auto',
    },
  },
  messageSuccessRate: {
    title: 'Message Success Rate',
    type: 'stat',
    targets: [
      {
        expr: 'sum(rate(ws_messages_total{result="success"}[5m])) / sum(rate(ws_messages_total[5m]))',
        legendFormat: 'Success Rate',
        refId: 'A',
      },
    ],
    fieldConfig: {
      defaults: {
        color: {
          mode: 'thresholds',
        },
        mappings: [],
        max: 1,
        min: 0,
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'red',
              value: null,
            },
            {
              color: 'yellow',
              value: 0.9,
            },
            {
              color: 'green',
              value: 0.95,
            },
          ],
        },
        unit: 'percentunit',
      },
    },
    options: {
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
  },
  broadcastSuccessRate: {
    title: 'Broadcast Success Rate',
    type: 'stat',
    targets: [
      {
        expr: 'sum(rate(ws_broadcast_operations_total{result="success"}[5m])) / sum(rate(ws_broadcast_operations_total[5m]))',
        legendFormat: 'Success Rate',
        refId: 'A',
      },
    ],
    fieldConfig: {
      defaults: {
        color: {
          mode: 'thresholds',
        },
        mappings: [],
        max: 1,
        min: 0,
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'red',
              value: null,
            },
            {
              color: 'yellow',
              value: 0.9,
            },
            {
              color: 'green',
              value: 0.95,
            },
          ],
        },
        unit: 'percentunit',
      },
    },
    options: {
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
  },
  avgConnectionDuration: {
    title: 'Avg Connection Duration',
    type: 'stat',
    targets: [
      {
        expr: 'rate(ws_connection_duration_seconds_sum[5m]) / rate(ws_connection_duration_seconds_count[5m])',
        legendFormat: 'Average',
        refId: 'A',
      },
    ],
    fieldConfig: {
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
            {
              color: 'yellow',
              value: 300,
            },
            {
              color: 'red',
              value: 600,
            },
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
      reduceOptions: {
        calcs: ['lastNotNull'],
        fields: '',
        values: false,
      },
      textMode: 'auto',
    },
  },
  // Connection State Distribution
  wsConnectionStateDistribution: {
    title: 'Connection State Distribution',
    type: 'piechart',
    targets: [
      {
        expr: 'ws_connections_active{state="active"}',
        legendFormat: 'Active',
        refId: 'A',
      },
      {
        expr: 'ws_connections_active{state="stale"}',
        legendFormat: 'Stale',
        refId: 'B',
      },
      {
        expr: 'ws_connections_active{state="disconnected"}',
        legendFormat: 'Disconnected',
        refId: 'C',
      },
    ],
    options: {
      reduceOptions: {
        values: false,
        calcs: ['lastNotNull'],
        fields: '',
      },
      pieType: 'pie',
      tooltip: {
        mode: 'single',
        sort: 'none',
      },
      legend: {
        displayMode: 'list',
        placement: 'bottom',
      },
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'palette-classic',
        },
        custom: {
          hideFrom: {
            legend: false,
            tooltip: false,
            vis: false,
          },
        },
        mappings: [],
      },
    },
  },
  // Message Type Distribution
  wsMessageTypeDistribution: {
    title: 'Message Type Distribution (Last 5m)',
    type: 'piechart',
    targets: [
      {
        expr: 'increase(ws_messages_total[5m]) by (type)',
        legendFormat: '{{type}}',
        refId: 'A',
      },
    ],
    options: {
      reduceOptions: {
        values: false,
        calcs: ['lastNotNull'],
        fields: '',
      },
      pieType: 'pie',
      tooltip: {
        mode: 'single',
        sort: 'none',
      },
      legend: {
        displayMode: 'list',
        placement: 'bottom',
      },
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'palette-classic',
        },
        custom: {
          hideFrom: {
            legend: false,
            tooltip: false,
            vis: false,
          },
        },
        mappings: [],
      },
    },
  },
  // Top Error Types Table
  wsTopErrors: {
    title: 'Top Error Types (Last Hour)',
    type: 'table',
    targets: [
      {
        expr: 'topk(10, increase(ws_errors_total[1h])) by (error_type, component)',
        legendFormat: '{{error_type}} - {{component}}',
        refId: 'A',
        format: 'table',
        instant: true,
      },
    ],
    fieldConfig: {
      defaults: {
        color: {
          mode: 'thresholds',
        },
        custom: {
          align: 'auto',
          displayMode: 'auto',
          inspect: false,
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
              value: 10,
            },
          ],
        },
      },
    },
    options: {
      showHeader: true,
    },
    transformations: [
      {
        id: 'organize',
        options: {
          excludeByName: {
            Time: true,
            '__name__': true,
            job: true,
            instance: true,
          },
          indexByName: {},
          renameByName: {
            component: 'Component',
            error_type: 'Error Type',
            Value: 'Count',
          },
        },
      },
    ],
  },

  // === Time Series Panels Start Here ===

  wsConnectionRate: {
    title: 'Connection Rate',
    type: 'timeseries',
    targets: [
      {
        expr: 'rate(ws_connections_total{outcome="created"}[5m])',
        legendFormat: 'Created/sec',
        refId: 'A',
      },
      {
        expr: 'rate(ws_connections_total{outcome="removed"}[5m])',
        legendFormat: 'Removed/sec',
        refId: 'B',
      },
    ],
    fieldConfig: {
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
        unit: 'reqps',
      },
    },
    options: {
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
  },
  wsConnectionDuration: {
    title: 'Connection Duration',
    type: 'timeseries',
    targets: [
      {
        expr: 'histogram_quantile(0.50, rate(ws_connection_duration_seconds_bucket[5m])) by (reason)',
        legendFormat: 'p50 - {{reason}}',
        refId: 'A',
      },
      {
        expr: 'histogram_quantile(0.95, rate(ws_connection_duration_seconds_bucket[5m])) by (reason)',
        legendFormat: 'p95 - {{reason}}',
        refId: 'B',
      },
      {
        expr: 'histogram_quantile(0.99, rate(ws_connection_duration_seconds_bucket[5m])) by (reason)',
        legendFormat: 'p99 - {{reason}}',
        refId: 'C',
      },
    ],
    fieldConfig: {
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
        unit: 's',
      },
    },
    options: {
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
  },
  // Message Processing Panels
  wsMessageRate: {
    title: 'Message Processing Rate',
    type: 'timeseries',
    targets: [
      {
        expr: 'rate(ws_messages_total{result="success"}[5m]) by (type)',
        legendFormat: 'Success - {{type}}',
        refId: 'A',
      },
      {
        expr: 'rate(ws_messages_total{result="failed"}[5m]) by (type)',
        legendFormat: 'Failed - {{type}}',
        refId: 'B',
      },
    ],
    fieldConfig: {
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
        unit: 'reqps',
      },
    },
    options: {
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
  },
  wsMessageProcessingDuration: {
    title: 'Message Processing Duration',
    type: 'timeseries',
    targets: [
      {
        expr: 'histogram_quantile(0.50, rate(ws_message_processing_duration_seconds_bucket[5m])) by (type, stage)',
        legendFormat: 'p50 - {{type}}/{{stage}}',
        refId: 'A',
      },
      {
        expr: 'histogram_quantile(0.95, rate(ws_message_processing_duration_seconds_bucket[5m])) by (type, stage)',
        legendFormat: 'p95 - {{type}}/{{stage}}',
        refId: 'B',
      },
    ],
    fieldConfig: {
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
        unit: 's',
      },
    },
    options: {
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
  },
  // Broadcast Panels
  wsBroadcastOperations: {
    title: 'Broadcast Operations',
    type: 'timeseries',
    targets: [
      {
        expr: 'rate(ws_broadcast_operations_total{result="success"}[5m]) by (event_type)',
        legendFormat: 'Success - {{event_type}}',
        refId: 'A',
      },
      {
        expr: 'rate(ws_broadcast_operations_total{result="failed"}[5m]) by (event_type)',
        legendFormat: 'Failed - {{event_type}}',
        refId: 'B',
      },
      {
        expr: 'rate(ws_broadcast_operations_total{result="no_subscribers"}[5m]) by (event_type)',
        legendFormat: 'No Subscribers - {{event_type}}',
        refId: 'C',
      },
    ],
    fieldConfig: {
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
        unit: 'ops',
      },
    },
    options: {
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
  },
  wsBroadcastDelivery: {
    title: 'Broadcast Message Delivery',
    type: 'timeseries',
    targets: [
      {
        expr: 'rate(ws_broadcast_delivery_total{outcome="delivered"}[5m]) by (event_type)',
        legendFormat: 'Delivered - {{event_type}}',
        refId: 'A',
      },
      {
        expr: 'rate(ws_broadcast_delivery_total{outcome="failed"}[5m]) by (event_type)',
        legendFormat: 'Failed - {{event_type}}',
        refId: 'B',
      },
    ],
    fieldConfig: {
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
        unit: 'msgps',
      },
    },
    options: {
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
  },
  wsBroadcastDuration: {
    title: 'Broadcast Duration',
    type: 'timeseries',
    targets: [
      {
        expr: 'histogram_quantile(0.50, rate(ws_broadcast_duration_seconds_bucket[5m])) by (event_type)',
        legendFormat: 'p50 - {{event_type}}',
        refId: 'A',
      },
      {
        expr: 'histogram_quantile(0.95, rate(ws_broadcast_duration_seconds_bucket[5m])) by (event_type)',
        legendFormat: 'p95 - {{event_type}}',
        refId: 'B',
      },
      {
        expr: 'histogram_quantile(0.99, rate(ws_broadcast_duration_seconds_bucket[5m])) by (event_type)',
        legendFormat: 'p99 - {{event_type}}',
        refId: 'C',
      },
    ],
    fieldConfig: {
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
        unit: 's',
      },
    },
    options: {
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
  },
  // Subscription Panels
  wsActiveSubscriptions: {
    title: 'Active Subscriptions by Event Type',
    type: 'timeseries',
    targets: [
      {
        expr: 'ws_subscriptions_active by (event_type)',
        legendFormat: '{{event_type}}',
        refId: 'A',
      },
    ],
    fieldConfig: {
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
            mode: 'normal',
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
        unit: 'short',
      },
    },
    options: {
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
  },
  wsSubscriptionOperations: {
    title: 'Subscription Operations',
    type: 'timeseries',
    targets: [
      {
        expr: 'rate(ws_subscription_operations_total{operation="subscribe"}[5m]) by (event_type)',
        legendFormat: 'Subscribe - {{event_type}}',
        refId: 'A',
      },
      {
        expr: 'rate(ws_subscription_operations_total{operation="unsubscribe"}[5m]) by (event_type)',
        legendFormat: 'Unsubscribe - {{event_type}}',
        refId: 'B',
      },
    ],
    fieldConfig: {
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
        unit: 'ops',
      },
    },
    options: {
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
  },
  // Health and Monitoring Panels
  wsInvariantViolations: {
    title: 'Invariant Violations',
    type: 'stat',
    targets: [
      {
        expr: 'increase(ws_invariant_violations_total[1h]) by (invariant_type)',
        legendFormat: '{{invariant_type}}',
        refId: 'A',
      },
    ],
    fieldConfig: {
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
            {
              color: 'yellow',
              value: 1,
            },
            {
              color: 'red',
              value: 5,
            },
          ],
        },
        unit: 'short',
      },
    },
    options: {
      colorMode: 'background',
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
  },
  wsHealthChecks: {
    title: 'Health Check Success Rate',
    type: 'timeseries',
    targets: [
      {
        expr: 'rate(ws_health_checks_total{result="success"}[5m]) / rate(ws_health_checks_total[5m]) by (check_type)',
        legendFormat: '{{check_type}}',
        refId: 'A',
      },
    ],
    fieldConfig: {
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
        max: 1,
        min: 0,
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
            {
              color: 'red',
              value: 0.9,
            },
          ],
        },
        unit: 'percentunit',
      },
    },
    options: {
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
  },
  wsResourceUsage: {
    title: 'Resource Usage',
    type: 'timeseries',
    targets: [
      {
        expr: 'ws_resource_usage{resource_type="active_connections"}',
        legendFormat: 'Active Connections',
        refId: 'A',
      },
      {
        expr: 'ws_resource_usage{resource_type="total_connections"}',
        legendFormat: 'Total Connections',
        refId: 'B',
      },
      {
        expr: 'ws_resource_usage{resource_type="pending_messages"}',
        legendFormat: 'Pending Messages',
        refId: 'C',
      },
    ],
    fieldConfig: {
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
        unit: 'short',
      },
    },
    options: {
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
  },
  // Error Panels
  wsErrors: {
    title: 'WebSocket Errors',
    type: 'timeseries',
    targets: [
      {
        expr: 'rate(ws_errors_total[5m]) by (error_type, component)',
        legendFormat: '{{component}}/{{error_type}}',
        refId: 'A',
      },
    ],
    fieldConfig: {
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
        unit: 'eps',
      },
    },
    options: {
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
  },
  wsSystemEvents: {
    title: 'System Events',
    type: 'timeseries',
    targets: [
      {
        expr: 'rate(ws_system_events_total[5m]) by (event_type)',
        legendFormat: '{{event_type}}',
        refId: 'A',
      },
    ],
    fieldConfig: {
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
        unit: 'eps',
      },
    },
    options: {
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
  },
  // Summary Stat Panels
  totalMessages: {
    title: 'Total Messages Processed',
    type: 'stat',
    targets: [
      {
        expr: 'sum(ws_messages_total)',
        legendFormat: 'Total',
        refId: 'A',
      },
    ],
    fieldConfig: {
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
            {
              color: 'red',
              value: 10000,
            },
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
      reduceOptions: {
        calcs: ['lastNotNull'],
        fields: '',
        values: false,
      },
      textMode: 'auto',
    },
  },
  totalBroadcasts: {
    title: 'Total Broadcasts',
    type: 'stat',
    targets: [
      {
        expr: 'sum(ws_broadcast_operations_total)',
        legendFormat: 'Total',
        refId: 'A',
      },
    ],
    fieldConfig: {
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
            {
              color: 'red',
              value: 1000,
            },
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
      reduceOptions: {
        calcs: ['lastNotNull'],
        fields: '',
        values: false,
      },
      textMode: 'auto',
    },
  },
}

