local panels = import 'lib/ws-panels.libsonnet';
local utils = import 'lib/ws-utils.libsonnet';

{
  annotations: {
    list: [
      {
        builtIn: 1,
        datasource: {
          type: 'datasource',
          uid: 'ws_grafana',
        },
        enable: true,
        hide: true,
        iconColor: 'rgba(0, 211, 255, 1)',
        name: 'Annotations & Alerts',
        type: 'dashboard',
      },
    ],
  },
  editable: true,
  fiscalYearStartMonth: 0,
  graphTooltip: 1,
  id: null,
  links: [],
  liveNow: false,
  panels: [
    // Row 1: Connection Overview
    utils.row('Connection Overview', 0, 0) { id: 1 },
    panels.wsConnectionsActive { gridPos: utils.gridPos(0, 1, 6, 8), id: 2 },
    panels.wsConnectionRate { gridPos: utils.gridPos(6, 1, 9, 8), id: 3 },
    panels.wsConnectionDuration { gridPos: utils.gridPos(15, 1, 9, 8), id: 4 },

    // Row 2: Connection Stats and Distribution
    panels.wsConnectionStateDistribution { gridPos: utils.gridPos(0, 9, 8, 6), id: 5 },
    panels.avgConnectionDuration { gridPos: utils.gridPos(8, 9, 4, 3), id: 10 },
    panels.wsClientConnections { gridPos: utils.gridPos(12, 9, 12, 6), id: 11 },

    // Row 3: Message Processing
    utils.row('Message Processing', 0, 15) { id: 12 },
    panels.wsMessageRate { gridPos: utils.gridPos(0, 16, 12, 8), id: 13 },
    panels.wsSubscriptionOperations { gridPos: utils.gridPos(12, 16, 12, 8), id: 14 },

    // Row 4: Subscriptions
    panels.wsActiveSubscriptions { gridPos: utils.gridPos(0, 24, 12, 8), id: 15 },

    // Row 5: Monitoring & Errors
    utils.row('Monitoring & Errors', 0, 32) { id: 16 },
    panels.wsTimeoutMonitorOperations { gridPos: utils.gridPos(0, 33, 8, 8), id: 17 },
    panels.wsErrors { gridPos: utils.gridPos(8, 33, 16, 8), id: 18 },

    // Row 6: Error Analysis
    utils.row('Error Analysis', 0, 41) { id: 19 },
    {
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
          thresholds: { mode: 'absolute', steps: [{ value: null, color: 'green' }, { value: 5, color: 'red' }] },
        },
        overrides: [],
      },
      options: { showHeader: true, displayMode: 'color-text' },
      transformations: [
        { id: 'organize', options: { excludeByName: { Time: true, __name__: true }, renameByName: { Value: 'Count' } } },
      ],
      gridPos: utils.gridPos(0, 42, 24, 10),
      id: 20,
    },
  ],
  refresh: '5s',
  schemaVersion: 38,
  tags: ['rust', 'websocket', 'prometheus', 'tokio'],
  templating: {
    list: [
      utils.templateVars.instance {
        current: {
          selected: true,
          text: 'All',
          value: '$__all',
        },
        hide: 0,
        includeAll: true,
        label: 'Instance',
        multi: true,
        name: 'instance',
        options: [],
        query: 'label_values(ws_connection_lifecycle_total, instance)',
        queryType: '',
        refresh: 1,
        regex: '',
        skipUrlSync: false,
        sort: 1,
        type: 'query',
      },
      utils.templateVars.event_type {
        current: {
          selected: true,
          text: 'All',
          value: '$__all',
        },
        hide: 0,
        includeAll: true,
        label: 'Event Type',
        multi: true,
        name: 'event_type',
        options: [],
        query: 'label_values(ws_connection_subscriptions, event_type)',
        queryType: '',
        refresh: 1,
        regex: '',
        skipUrlSync: false,
        sort: 1,
        type: 'query',
      },
    ],
  },
  time: {
    from: 'now-15m',
    to: 'now',
  },
  timepicker: {
    refresh_intervals: [
      '5s',
      '10s',
      '30s',
      '1m',
      '5m',
      '15m',
      '30m',
      '1h',
      '2h',
      '1d',
    ],
  },
  timezone: '',
  title: 'WebSocket Metrics Dashboard',
  uid: 'websocket-dashboard-v2',
  version: 1,
  weekStart: '',
}
