local panels = import 'lib/panels.libsonnet';
local utils = import 'lib/utils.libsonnet';
local panelDefaults = import 'lib/panel-defaults.libsonnet';
local health = import 'lib/health-panels.libsonnet';
local clients = import 'lib/clients-panels.libsonnet';
local abuse = import 'lib/abuse-panels.libsonnet';

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

    // =============== CLIENTS (#214/#229) ===============
    // Is anybody there, and is anyone holding a socket for nothing. WS
    // CONNS/LIVE/SUBSCRIBED are read as a triple — see
    // clients-panels.libsonnet's header — so they sit adjacent rather than
    // in separate rows the way HEALTH's independent conditions do.
    clients.wsConns { gridPos: utils.gridPos(0, 18, 6, 4), id: 910 },
    clients.live { gridPos: utils.gridPos(6, 18, 6, 4), id: 911 },
    clients.subscribed { gridPos: utils.gridPos(12, 18, 6, 4), id: 912 },
    clients.reqPerMin { gridPos: utils.gridPos(18, 18, 6, 4), id: 913 },

    clients.churn { gridPos: utils.gridPos(0, 22, 12, 6), id: 914 },
    clients.closesByReason { gridPos: utils.gridPos(12, 22, 12, 6), id: 915 },

    clients.cacheHitMissByNamespace { gridPos: utils.gridPos(0, 28, 12, 6), id: 916 },
    clients.sqlitePool { gridPos: utils.gridPos(12, 28, 12, 6), id: 917 },

    // WS internals — see clients-panels.libsonnet's own comment: readers
    // for the families #226 kept but that didn't make the epic's own
    // mock-up, reclaimed from `parked/ws-panels.libsonnet`.
    clients.guardOccupancy { gridPos: utils.gridPos(0, 34, 12, 6), id: 918 },
    clients.clientTypeDistribution { gridPos: utils.gridPos(12, 34, 12, 6), id: 919 },
    clients.messageRate { gridPos: utils.gridPos(0, 40, 12, 6), id: 920 },
    clients.connectionErrors { gridPos: utils.gridPos(12, 40, 12, 6), id: 921 },

    // =============== ABUSE (#215/#232) ===============
    // "Are we abusing resources — do we close sockets, is the rate limiter
    // doing its job." Rebuilds `dashboards/parked/rate-limit.jsonnet`'s
    // "Proof of Failure (By Contradiction)" idea on premises that now exist
    // — see abuse-panels.libsonnet's own header.
    abuse.rateLimited { gridPos: utils.gridPos(0, 46, 6, 4), id: 922 },
    abuse.shed { gridPos: utils.gridPos(6, 46, 6, 4), id: 923 },
    abuse.timedOut { gridPos: utils.gridPos(12, 46, 6, 4), id: 924 },
    abuse.wsRefused { gridPos: utils.gridPos(18, 46, 6, 4), id: 925 },

    abuse.refusalsByReason { gridPos: utils.gridPos(0, 50, 12, 6), id: 926 },
    abuse.tokensAvailable { gridPos: utils.gridPos(12, 50, 12, 6), id: 927 },

    abuse.invariant { gridPos: utils.gridPos(0, 56, 24, 4), id: 928 },

    // =============== ROW 1: UPTIME SLA ===============
    panels.uptimeOverallStatus { gridPos: utils.gridPos(0, 60, 6, 4) },
    panels.uptimeSLA30d { gridPos: utils.gridPos(6, 60, 6, 4) },
    panels.tcpConnectivity { gridPos: utils.gridPos(12, 60, 6, 4) },
    panels.httpWebSocketProbe { gridPos: utils.gridPos(18, 60, 6, 4) },

    // =============== ROW 2: UPTIME TRENDS & DIAGNOSTICS ===============
    panels.uptimeTrend7d { gridPos: utils.gridPos(0, 64, 12, 8) },
    panels.probeDiagnostics { gridPos: utils.gridPos(12, 64, 12, 8) },

    // =============== ROW 3: APPLICATION METRICS ===============
    // The standalone liveness stat this row used to carry (#212/G3) is
    // superseded by the HEALTH row's UP panel above, which is the same
    // `up{job="file_host"}` query — no need for both.
    panels.operationDuration { gridPos: utils.gridPos(0, 72, 24, 8) },
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
