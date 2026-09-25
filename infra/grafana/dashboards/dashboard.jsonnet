local panels = import 'lib/panels.libsonnet';
local utils = import 'lib/utils.libsonnet';
local panelDefaults = import 'lib/panel-defaults.libsonnet';
local health = import 'lib/health-panels.libsonnet';
local clients = import 'lib/clients-panels.libsonnet';
local nudge = import 'lib/nudge-panels.libsonnet';
local abuse = import 'lib/abuse-panels.libsonnet';
local overview = import 'lib/overview-panels.libsonnet';

local row(title, y, id) = { type: 'row', title: title, id: id, collapsed: false, gridPos: utils.gridPos(0, y, 24, 1), panels: [] };

// A collapsed row carries its panels inside itself (that's how Grafana
// stores one); `hardenAll` is applied to them here because the dashboard-level
// `hardenAll` below only walks the top-level list.
local collapsedRow(title, y, id, children) = row(title, y, id) { collapsed: true, panels: panelDefaults.hardenAll(children) };

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
        // Scoped to file_host: every exporter exports this gauge too, so
        // unscoped it annotated each exporter's restart as file_host's.
        expr: 'changes(process_start_time_seconds{job="file_host"}[$__interval]) > 0',
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
  // Top to bottom in the order an operator asks: is it broken (HEALTH), how
  // is it doing (GOLDEN SIGNALS), are the other services up and were they
  // (SERVICES) — all on the first screen, no scrolling. Below that, one
  // uncollapsed section each for traffic/latency/errors, the metal and
  // realtime; the drill-downs nobody needs at a glance are collapsed rows,
  // which Grafana doesn't even query until opened.
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

    // =============== GOLDEN SIGNALS ===============
    overview.traffic { gridPos: utils.gridPos(0, 4, 4, 4), id: 940 },
    overview.wsClients { gridPos: utils.gridPos(4, 4, 4, 4), id: 941 },
    overview.latencyP95 { gridPos: utils.gridPos(8, 4, 4, 4), id: 942 },
    overview.availability { gridPos: utils.gridPos(12, 4, 4, 4), id: 943 },
    overview.restarts { gridPos: utils.gridPos(16, 4, 4, 4), id: 944 },
    overview.upFor { gridPos: utils.gridPos(20, 4, 4, 4), id: 945 },

    // =============== SERVICES ===============
    overview.serviceTimeline { gridPos: utils.gridPos(0, 8, 24, 6), id: 946 },

    row('Traffic, latency & errors', 14, 960),
    health.requestRateByOutcome { gridPos: utils.gridPos(0, 15, 8, 8) },
    health.httpLatencyByRoute { gridPos: utils.gridPos(8, 15, 8, 8) },
    panels.tracingErrors { gridPos: utils.gridPos(16, 15, 8, 8) },

    row('Metal — what the services cost the host', 23, 961),
    overview.cpuVsLimit { gridPos: utils.gridPos(0, 24, 6, 7), id: 947 },
    overview.memVsLimit { gridPos: utils.gridPos(6, 24, 6, 7), id: 948 },
    overview.diskIo { gridPos: utils.gridPos(12, 24, 6, 7), id: 949 },
    overview.diskSpace { gridPos: utils.gridPos(18, 24, 6, 7), id: 950 },

    // =============== REALTIME (#214/#229) ===============
    // WS internals — see clients-panels.libsonnet's own comment: readers
    // for the families #226 kept, reclaimed from `parked/ws-panels.libsonnet`.
    row('Realtime — WebSocket', 31, 962),
    clients.clientTypeDistribution { gridPos: utils.gridPos(0, 32, 12, 6), id: 919 },
    clients.guardOccupancy { gridPos: utils.gridPos(12, 32, 12, 6), id: 918 },
    clients.messageRate { gridPos: utils.gridPos(0, 38, 12, 6), id: 920 },
    clients.connectionErrors { gridPos: utils.gridPos(12, 38, 12, 6), id: 921 },

    // =============== DRILL-DOWNS (collapsed) ===============
    // Connection lifecycle: WS CONNS / LIVE / SUBSCRIBED are read as a
    // triple, and DEVICE CONNS / PROBE split WS CONNS a second way — see
    // clients-panels.libsonnet's header. LEAKED ENTRIES is what the gap
    // between those two splits means.
    collapsedRow('WebSocket connection lifecycle', 44, 963, [
      overview.leakedEntries { gridPos: utils.gridPos(0, 45, 4, 4), id: 951 },
      clients.wsConns { gridPos: utils.gridPos(4, 45, 4, 4), id: 910 },
      clients.deviceConns { gridPos: utils.gridPos(8, 45, 4, 4), id: 929 },
      clients.probeConns { gridPos: utils.gridPos(12, 45, 4, 4), id: 930 },
      clients.live { gridPos: utils.gridPos(16, 45, 4, 4), id: 911 },
      clients.subscribed { gridPos: utils.gridPos(20, 45, 4, 4), id: 912 },
      clients.churn { gridPos: utils.gridPos(0, 49, 12, 6), id: 914 },
      clients.closesByReason { gridPos: utils.gridPos(12, 49, 12, 6), id: 915 },
    ]),

    // ABUSE (#215/#232): "are we abusing resources — do we close sockets,
    // is the rate limiter doing its job." See abuse-panels.libsonnet.
    collapsedRow('Rate limiting & refusals', 45, 964, [
      abuse.rateLimited { gridPos: utils.gridPos(0, 46, 6, 4), id: 922 },
      abuse.shed { gridPos: utils.gridPos(6, 46, 6, 4), id: 923 },
      abuse.timedOut { gridPos: utils.gridPos(12, 46, 6, 4), id: 924 },
      abuse.wsRefused { gridPos: utils.gridPos(18, 46, 6, 4), id: 925 },
      abuse.refusalsByReason { gridPos: utils.gridPos(0, 50, 12, 6), id: 926 },
      abuse.tokensAvailable { gridPos: utils.gridPos(12, 50, 12, 6), id: 927 },
      abuse.invariant { gridPos: utils.gridPos(0, 56, 24, 4), id: 928 },
    ]),

    // Is the engagement waker's tick (LOOPS, above) actually landing
    // notifications, and if not, why not. See nudge-panels.libsonnet.
    collapsedRow('Nudge waker', 46, 965, [
      nudge.due { gridPos: utils.gridPos(0, 47, 4, 4), id: 931 },
      nudge.configErrors { gridPos: utils.gridPos(4, 47, 5, 4), id: 933 },
      nudge.outcomesByVerdict { gridPos: utils.gridPos(9, 47, 15, 4), id: 932 },
      nudge.passDuration { gridPos: utils.gridPos(0, 51, 24, 4), id: 934 },
    ]),

    collapsedRow('Cache, storage & operations', 47, 966, [
      clients.cacheHitMissByNamespace { gridPos: utils.gridPos(0, 48, 12, 6), id: 916 },
      clients.sqlitePool { gridPos: utils.gridPos(12, 48, 12, 6), id: 917 },
      panels.operationDuration { gridPos: utils.gridPos(0, 54, 12, 6) },
      panels.probeDuration { gridPos: utils.gridPos(12, 54, 12, 6), id: 11 },
    ]),
  ]),
  refresh: '5s',
  schemaVersion: 38,
  tags: ['rust', 'axum', 'prometheus', 'sla', 'health'],
  // Which build is running, as header pickers rather than a panel: always
  // visible, one line, and out of the way. Read from `service_info`'s
  // labels (build_info.rs) with `query_result`, which is an instant query at
  // the end of the time range — the build running *now*. `label_values`
  // would list every build seen anywhere in the range, so across a deploy
  // the header could keep showing the previous one.
  templating: {
    list: [
      {
        name: v.name,
        label: v.name,
        type: 'query',
        datasource: { type: 'prometheus', uid: 'prometheus' },
        definition: 'query_result(max by (%s) (service_info{job="file_host"}))' % v.label,
        query: { query: 'query_result(max by (%s) (service_info{job="file_host"}))' % v.label, refId: 'StandardVariableQuery' },
        regex: '/%s="([^"]+)"/' % v.label,
        refresh: 2,
        hide: 0,
        includeAll: false,
        multi: false,
        sort: 0,
        options: [],
        current: {},
      }
      for v in [{ name: 'build', label: 'git_sha' }, { name: 'built', label: 'built_at' }]
    ],
  },
  time: { from: 'now-1h', to: 'now' },
  timepicker: {},
  timezone: '',
  title: '🩺 file_host',
  uid: 'file-host-dashboard',
  version: 1,
};

dashboard
