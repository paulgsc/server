local panels = import 'lib/panels.libsonnet';
local utils = import 'lib/utils.libsonnet';

local dashboard = {
  annotations: {
    list: [
      {
        builtIn: 1,
        datasource: {
          type: 'datasource',
          uid: 'P00000000',
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
  id: 1,
  links: [],
  liveNow: false,
  panels: [
    // Row 1: Request Rate and HTTP Latency
    panels.httpRequestRate { gridPos: utils.gridPos(0, 0, 12, 8) },
    panels.httpLatency { gridPos: utils.gridPos(12, 0, 12, 8) },

    // Row 2: Operation Duration and Cache Operations
    panels.operationDuration { gridPos: utils.gridPos(0, 8, 12, 8) },
    panels.cacheHitsMisses { gridPos: utils.gridPos(12, 8, 12, 8) },

    // Row 3: Stats panels
    panels.totalRequests { gridPos: utils.gridPos(0, 16, 6, 4) },
    panels.totalCacheOps { gridPos: utils.gridPos(6, 16, 6, 4) },
    panels.cacheHitRate { gridPos: utils.gridPos(12, 16, 6, 4) },
    panels.rateLimitedRequests { gridPos: utils.gridPos(18, 16, 6, 4) },
  ],
  refresh: '5s',
  schemaVersion: 38,
  tags: ['rust', 'axum', 'prometheus'],
  templating: {
    list: [],
  },
  time: {
    from: 'now-15m',
    to: 'now',
  },
  timepicker: {},
  timezone: '',
  title: 'Some Metrics',
  uid: 'file-host-dashboard',
  version: 1,
  weekStart: '',
};

dashboard
