local panels = import 'lib/panels.libsonnet';
local utils = import 'lib/utils.libsonnet';
local panelDefaults = import 'lib/panel-defaults.libsonnet';
local health = import 'lib/health-panels.libsonnet';

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
      {
        // #213/F4: "add a dashboard annotation on restarts, so a red panel
        // can be attributed to a deploy at a glance." `process_start_time_seconds`
        // (build_info::record) is set once at startup and holds constant
        // for the process's life, so `changes()` — which counts any change
        // within the window, not just increases — fires exactly once per
        // restart, the instant it jumps to the new start time.
        datasource: { type: 'prometheus', uid: 'prometheus' },
        enable: true,
        expr: 'changes(process_start_time_seconds[$__interval]) > 0',
        iconColor: 'purple',
        name: 'Restarts',
        step: '60s',
        titleFormat: 'file_host restarted',
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
    // =============== HEALTH (#213/#225) ===============
    // Six conditions, six panels, ordered by diagnostic precedence — UP
    // before DEPS before the rest, so the leftmost red is the one to
    // investigate. See docs/fault-conditions.md; each panel links to its
    // own condition there.
    health.up { gridPos: utils.gridPos(0, 0, 4, 4) },
    health.deps { gridPos: utils.gridPos(4, 0, 4, 4) },
    health.errors { gridPos: utils.gridPos(8, 0, 4, 4) },
    health.refusals { gridPos: utils.gridPos(12, 0, 4, 4) },
    health.loops { gridPos: utils.gridPos(16, 0, 4, 4) },
    health.signal { gridPos: utils.gridPos(20, 0, 4, 4) },

    // Which build this is, and how long it's been running — see
    // health-panels.libsonnet's own comment on `buildInfo`/`uptime`.
    health.buildInfo { gridPos: utils.gridPos(0, 4, 18, 2) },
    health.uptime { gridPos: utils.gridPos(18, 4, 6, 2) },

    health.requestRateByOutcome { gridPos: utils.gridPos(0, 6, 24, 6) },
    health.httpLatencyByRoute { gridPos: utils.gridPos(0, 12, 24, 6) },

    // =============== ROW 1: UPTIME SLA ===============
    panels.uptimeOverallStatus { gridPos: utils.gridPos(0, 18, 6, 4) },
    panels.uptimeSLA30d { gridPos: utils.gridPos(6, 18, 6, 4) },
    panels.tcpConnectivity { gridPos: utils.gridPos(12, 18, 6, 4) },
    panels.httpWebSocketProbe { gridPos: utils.gridPos(18, 18, 6, 4) },

    // =============== ROW 2: UPTIME TRENDS & DIAGNOSTICS ===============
    panels.uptimeTrend7d { gridPos: utils.gridPos(0, 22, 12, 8) },
    panels.probeDiagnostics { gridPos: utils.gridPos(12, 22, 12, 8) },

    // =============== ROW 3: APPLICATION METRICS ===============
    // The standalone liveness stat this row used to carry (#212/G3) is
    // superseded by the HEALTH row's UP panel above, which is the same
    // `up{job="file_host"}` query — no need for both.
    panels.operationDuration { gridPos: utils.gridPos(0, 30, 24, 8) },
  ]),
  refresh: '5s',
  schemaVersion: 38,
  tags: ['rust', 'axum', 'prometheus', 'sla', 'health'],
  templating: { list: [] },
  time: { from: 'now-1h', to: 'now' },
  timepicker: {},
  timezone: '',
  title: '🩺 file_host Operational Dashboard',
  uid: 'file-host-dashboard',
  version: 1,
};

dashboard
