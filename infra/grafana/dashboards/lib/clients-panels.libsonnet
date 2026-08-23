// clients-panels.libsonnet
//
// The CLIENTS row — #214/#229 (C4). "Is anybody there, and is anyone
// holding a socket for nothing." Second row of the operational dashboard,
// below HEALTH.
//
// WS CONNS / LIVE / SUBSCRIBED are meant to be read together, not each
// against its own threshold — `3/3/3` is healthy, `3/3/0` is three sockets
// pooling for nobody, `3/1/3` is two half-open connections not yet reaped.
// DEVICE CONNS / PROBE split WS CONNS itself the same way, for the fourth
// reading that triple can't express: `3/3/3` next to `PROBE: 5` means the
// three real sockets are fine and the blackbox WS liveness probe accounts
// for the rest of WS CONNS, not a mystery. All six stats therefore share
// one always-green baseline (`harden()`, applied where this is imported
// into `dashboard.jsonnet`, is what turns a genuinely absent series grey
// instead) rather than a red ceiling copied from a deployment this one
// isn't — see #219 and #229's own note that a threshold sized for a public
// service would never fire here.
local cachePanels = import 'cache/cache-panels.libsonnet';
local panelDefaults = import 'panel-defaults.libsonnet';

local alwaysGreen = {
  mode: 'thresholds',
  steps: [{ color: 'green', value: null }],
};

local statFieldConfig(unit) = {
  defaults: {
    unit: unit,
    color: { mode: 'thresholds' },
    thresholds: alwaysGreen,
    noValue: 'no data',
  },
  overrides: [],
};

local statOptions = {
  colorMode: 'value',
  graphMode: 'none',
  justifyMode: 'center',
  orientation: 'horizontal',
  reduceOptions: { calcs: ['lastNotNull'], values: false },
  textMode: 'value',
};

local timeSeriesFieldConfig = {
  defaults: {
    custom: { drawStyle: 'line', fillOpacity: 15, lineWidth: 2, pointSize: 4, showPoints: 'never', spanNulls: false, stacking: { mode: 'none' } },
    color: { mode: 'palette-classic' },
  },
  overrides: [],
};

local timeSeriesOptions = {
  legend: { showLegend: true, placement: 'bottom' },
  tooltip: { mode: 'multi', sort: 'desc' },
};

{
  // WS CONNS — `connected`, the raw store size. Updated synchronously from
  // `add_connection`/`remove_connection` (file_host's `websocket.rs`), so
  // this one moves within a scrape interval; LIVE/SUBSCRIBED move on
  // `metrics::periodic`'s own cadence (10s) instead — see #227.
  wsConns: {
    title: 'WS CONNS',
    type: 'stat',
    targets: [{ expr: 'sum(ws_connections{state="connected"})', instant: true, refId: 'A' }],
    fieldConfig: statFieldConfig('none'),
    options: statOptions,
  },

  // DEVICE CONNS — WS CONNS with the blackbox WS liveness probe's own share
  // subtracted. `infra/blackbox.yml`'s `websocket_blackbox_http` job
  // completes a real upgrade against `/ws` on every 15s scrape and hangs up
  // without sending a frame; the server counts that like any other
  // connection until `STALE_TIMEOUT` (120s) reaps it, so up to ~8 can be
  // stacked up with nobody on the other end. `connection.rs::client_id_from_request`
  // tags that traffic `client_type="probe"` (via the header the blackbox
  // module now sends) precisely so this number can answer what WS CONNS
  // alone cannot: how many of those sockets are an actual device. Not a
  // replacement for WS CONNS — that stays the honest raw total — this is
  // the number to actually glance at.
  deviceConns: {
    title: 'DEVICE CONNS',
    type: 'stat',
    targets: [{ expr: 'sum(ws_connections{state="connected"}) - sum(ws_client_connections{client_type="probe"})', instant: true, refId: 'A' }],
    fieldConfig: statFieldConfig('none'),
    options: statOptions,
  },

  // PROBE — the blackbox WS liveness check's own share of WS CONNS, broken
  // out rather than left to be mistaken for a mystery device. Same
  // `ws_client_connections{client_type}` family `clientTypeDistribution`
  // already reads below, surfaced here where the top-line count actually
  // gets read.
  probeConns: {
    title: 'PROBE',
    type: 'stat',
    targets: [{ expr: 'sum(ws_client_connections{client_type="probe"})', instant: true, refId: 'A' }],
    fieldConfig: statFieldConfig('none'),
    options: statOptions,
  },

  // LIVE — active, not stale, heard from within `STALE_TIMEOUT`. The same
  // freshness window the WS message loop itself uses to decide staleness,
  // so this number and the connections it actually closes agree.
  live: {
    title: 'LIVE',
    type: 'stat',
    targets: [{ expr: 'sum(ws_connections{state="live"})', instant: true, refId: 'A' }],
    fieldConfig: statFieldConfig('none'),
    options: statOptions,
  },

  // SUBSCRIBED — connections with at least one subscription. `WS CONNS >
  // SUBSCRIBED`, sustained, is the "sockets pooling for nobody" signature
  // #227 exists to make visible.
  subscribed: {
    title: 'SUBSCRIBED',
    type: 'stat',
    targets: [{ expr: 'sum(ws_connections{state="subscribed"})', instant: true, refId: 'A' }],
    fieldConfig: statFieldConfig('none'),
    options: statOptions,
  },

  // REQ/MIN — the HEALTH row already answers "is the server rejecting or
  // saturated"; this is the plain-language "is anything happening at all"
  // companion, on the same `http_requests_total` HEALTH already emits.
  reqPerMin: {
    title: 'REQ/MIN',
    type: 'stat',
    targets: [{ expr: 'sum(rate(http_requests_total[5m])) * 60', instant: true, refId: 'A' }],
    fieldConfig: statFieldConfig('none') { defaults+: { decimals: 1 } },
    options: statOptions,
  },

  // Connection churn — opens/sec against closes/sec. A reconnect storm
  // shows as both rates rising together while WS CONNS itself stays flat,
  // which is otherwise close to invisible.
  churn: {
    title: 'Connection Churn: Open / Close',
    type: 'timeseries',
    targets: [
      { expr: 'sum(rate(ws_connection_lifecycle_total{event="created"}[5m]))', legendFormat: 'opened', refId: 'A' },
      { expr: 'sum(rate(ws_connection_lifecycle_total{event="removed"}[5m]))', legendFormat: 'closed', refId: 'B' },
    ],
    fieldConfig: timeSeriesFieldConfig { defaults+: { unit: 'ops' } },
    options: timeSeriesOptions,
  },

  // Closes by reason — `ws_connection_duration_seconds_count` is the same
  // histogram #227 records `end_reason` on at the `ConnectionCleanup` seam;
  // its `_count` series by `end_reason` *is* "why are my connections
  // disappearing", with no separate counter needed to answer it.
  closesByReason: {
    title: 'Closes by Reason',
    type: 'timeseries',
    targets: [{ expr: 'sum by (end_reason) (rate(ws_connection_duration_seconds_count[5m]))', legendFormat: '{{end_reason}}', refId: 'A' }],
    fieldConfig: timeSeriesFieldConfig { defaults+: { unit: 'ops', custom+: { stacking: { mode: 'normal' } } } },
    options: timeSeriesOptions,
  },

  // Cache hit/miss by namespace — #228's reader for `cache_hits_total`/
  // `cache_misses_total`, reused rather than redefined so this panel and
  // `cache-dashboard.json`'s can't drift onto two different queries for the
  // same two metrics.
  cacheHitMissByNamespace: cachePanels.cacheHitsAndMissesByNamespace,

  // SQLite pool: size vs idle — #228. Idle reaching zero while size sits at
  // the configured max (`main.rs`'s `max_connections(5)`) is pool
  // exhaustion, visible here *before* the 5s `busy_timeout` turns it into a
  // user-visible failure.
  sqlitePool: {
    title: 'SQLite Pool: Size vs Idle',
    type: 'timeseries',
    targets: [
      { expr: 'sqlite_pool_size', legendFormat: 'size', refId: 'A' },
      { expr: 'sqlite_pool_idle', legendFormat: 'idle', refId: 'B' },
    ],
    fieldConfig: timeSeriesFieldConfig { defaults+: { unit: 'none' } },
    options: timeSeriesOptions,
  },

  // --- WS internals ------------------------------------------------------
  // The four panels below aren't in the epic's own CLIENTS mock-up, but
  // each is the reader #217's contract check requires for a family #226/
  // #227 emit — and #229's own task list says to reclaim what survives from
  // `ws-dashboard.jsonnet` rather than leave those families readerless a
  // second time. `parked/ws-panels.libsonnet` designed the shapes; these
  // just point them at the metrics #226 kept.

  // `ConnectionGuard` occupancy vs its configured capacity — #227's
  // permit-leak assertion made visible: a leaked permit shows as occupancy
  // that doesn't return to zero even after `connected` does.
  guardOccupancy: {
    title: 'Connection Guard: Occupancy vs Capacity',
    type: 'timeseries',
    targets: [
      { expr: 'ws_connection_guard_occupancy', legendFormat: 'occupancy', refId: 'A' },
      { expr: 'ws_connection_guard_capacity', legendFormat: 'capacity', refId: 'B' },
    ],
    fieldConfig: timeSeriesFieldConfig { defaults+: { unit: 'none' } },
    options: timeSeriesOptions,
  },

  // Client type distribution — reclaimed from `parked/ws-panels.libsonnet`'s
  // `wsClientConnections`, pointed at the same `ws_client_connections`
  // family #226 kept (the `auth`/`proxy`/`direct` bucketing was already the
  // right granularity; see that story's non-goals).
  clientTypeDistribution: {
    title: 'Client Type Distribution',
    type: 'timeseries',
    targets: [{ expr: 'ws_client_connections', legendFormat: '{{client_type}}', refId: 'A' }],
    fieldConfig: timeSeriesFieldConfig { defaults+: { unit: 'none', custom+: { stacking: { mode: 'normal' } } } },
    options: timeSeriesOptions,
  },

  // Message rate by raw frame kind — reclaimed from `wsMessageRate`;
  // `message_type` is now `text`/`ping`/`pong`/`close`/`binary` (#226
  // relabelled it off the raw WebSocket frame, not parsed event content —
  // see `instrument.rs`'s header for why).
  messageRate: {
    title: 'Message Rate by Frame Kind',
    type: 'timeseries',
    targets: [{ expr: 'sum(rate(ws_connection_messages_total[5m])) by (message_type)', legendFormat: '{{message_type}}', refId: 'A' }],
    fieldConfig: timeSeriesFieldConfig { defaults+: { unit: 'ops' } },
    options: timeSeriesOptions,
  },

  // Connection errors by type/phase — reclaimed from `wsErrors`.
  connectionErrors: {
    title: 'Connection Errors',
    type: 'timeseries',
    targets: [{ expr: 'sum(rate(ws_connection_errors_total[5m])) by (error_type, phase)', legendFormat: '{{error_type}} - {{phase}}', refId: 'A' }],
    fieldConfig: timeSeriesFieldConfig { defaults+: { unit: 'ops' } },
    options: timeSeriesOptions,
  },
}
