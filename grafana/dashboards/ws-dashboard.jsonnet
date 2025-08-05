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
    panels.totalMessages { gridPos: utils.gridPos(8, 9, 4, 3), id: 6 },
    panels.totalBroadcasts { gridPos: utils.gridPos(12, 9, 4, 3), id: 7 },
    panels.messageSuccessRate { gridPos: utils.gridPos(16, 9, 4, 3), id: 8 },
    panels.broadcastSuccessRate { gridPos: utils.gridPos(20, 9, 4, 3), id: 9 },
    panels.avgConnectionDuration { gridPos: utils.gridPos(8, 12, 4, 3), id: 10 },

    // Row 3: Message Processing
    utils.row('Message Processing', 0, 15) { id: 11 },
    panels.wsMessageRate { gridPos: utils.gridPos(0, 16, 12, 8), id: 12 },
    panels.wsMessageProcessingDuration { gridPos: utils.gridPos(12, 16, 12, 8), id: 13 },

    // Row 4: Message Type Distribution
    panels.wsMessageTypeDistribution { gridPos: utils.gridPos(0, 24, 12, 6), id: 14 },

    // Row 5: Broadcast Operations
    utils.row('Broadcast Operations', 0, 30) { id: 15 },
    panels.wsBroadcastOperations { gridPos: utils.gridPos(0, 31, 8, 8), id: 16 },
    panels.wsBroadcastDelivery { gridPos: utils.gridPos(8, 31, 8, 8), id: 17 },
    panels.wsBroadcastDuration { gridPos: utils.gridPos(16, 31, 8, 8), id: 18 },

    // Row 6: Subscriptions
    utils.row('Subscriptions', 0, 39) { id: 19 },
    panels.wsActiveSubscriptions { gridPos: utils.gridPos(0, 40, 12, 8), id: 20 },
    panels.wsSubscriptionOperations { gridPos: utils.gridPos(12, 40, 12, 8), id: 21 },

    // Row 7: Health Monitoring
    utils.row('Health & Monitoring', 0, 48) { id: 22 },
    panels.wsHealthChecks { gridPos: utils.gridPos(0, 49, 8, 8), id: 23 },
    panels.wsResourceUsage { gridPos: utils.gridPos(8, 49, 8, 8), id: 24 },
    panels.wsInvariantViolations { gridPos: utils.gridPos(16, 49, 8, 8), id: 25 },

    // Row 8: Error Analysis
    utils.row('Error Analysis', 0, 57) { id: 26 },
    panels.wsErrors { gridPos: utils.gridPos(0, 58, 12, 8), id: 27 },
    panels.wsSystemEvents { gridPos: utils.gridPos(12, 58, 12, 8), id: 28 },
    panels.wsTopErrors { gridPos: utils.gridPos(0, 66, 24, 10), id: 29 },
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
        query: 'label_values(ws_connections_total, instance)',
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
        query: 'label_values(ws_broadcast_operations_total, event_type)',
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
  uid: 'websocket-dashboard',
  version: 1,
  weekStart: '',
}
