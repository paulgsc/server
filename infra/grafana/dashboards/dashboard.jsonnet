local panels = import 'lib/panels.libsonnet';
local utils = import 'lib/utils.libsonnet';
local panelDefaults = import 'lib/panel-defaults.libsonnet';

local dashboard = {
  annotations: {
    list: [
      {
        builtIn: 1,
        // Grafana's own built-in "Annotations & Alerts" store, not
        // Prometheus — always present under this literal uid. See #218.
        datasource: { type: 'grafana', uid: '-- Grafana --' },
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
  panels: panelDefaults.hardenAll([
    // =============== ROW 1: UPTIME SLA (TOP PRIORITY) ===============
    panels.uptimeOverallStatus { gridPos: utils.gridPos(0, 0, 6, 4) },
    panels.uptimeSLA30d { gridPos: utils.gridPos(6, 0, 6, 4) },
    panels.tcpConnectivity { gridPos: utils.gridPos(12, 0, 6, 4) },
    panels.httpWebSocketProbe { gridPos: utils.gridPos(18, 0, 6, 4) },

    // =============== ROW 2: UPTIME TRENDS & DIAGNOSTICS ===============
    panels.uptimeTrend7d { gridPos: utils.gridPos(0, 4, 12, 8) },
    panels.probeDiagnostics { gridPos: utils.gridPos(12, 4, 12, 8) },

    // =============== ROW 3: APPLICATION METRICS ===============
    panels.operationDuration { gridPos: utils.gridPos(0, 12, 12, 8) },

    // #219: "no data everywhere" on this row should read as "file_host is
    // not being scraped", not as "operations dropped to zero".
    panelDefaults.livenessPanel('file_host liveness', ['file_host'], 15, utils.gridPos(12, 12, 12, 8)),
  ]),
  refresh: '10s',
  schemaVersion: 38,
  tags: ['rust', 'axum', 'prometheus', 'sla'],
  templating: { list: [] },
  time: { from: 'now-1h', to: 'now' },
  timepicker: {},
  timezone: '',
  title: '🚀 Service Uptime & Performance Dashboard',
  uid: 'file-host-dashboard',
  version: 1,
};

dashboard
