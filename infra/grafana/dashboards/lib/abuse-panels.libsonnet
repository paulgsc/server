// abuse-panels.libsonnet
//
// The ABUSE row — #215 (A3/#232). "Are we abusing resources — do we close
// sockets, is the rate limiter doing its job." Reclaims the best idea in
// `dashboards/parked/rate-limit.jsonnet` (titled "Rate Limiter - Proof of
// Failure (By Contradiction)"): traffic above the configured limit with zero
// rejections is a contradiction, and a dashboard that can only alert on
// rejections being zero — because zero is the normal case — has to alert on
// that *conditional on* enough traffic to require some. The parked version
// never evaluated: it queried `http_requests_total`, which #222 (F2) had not
// landed yet, so both premises were undefined and the panel read as "no
// contradiction found" for its entire existence. Both premises exist now:
// `http_requests_total` (#222) and `rate_limit_decisions_total` (#230).
//
// The four-stat row and the refusal/token panels below it are informational,
// not alerts — a working limiter *should* show non-zero RATE LIMITED under
// abusive traffic, so they stay always-green like CLIENTS' own stat row
// (#219's "no data" grey still applies via `panelDefaults.hardenAll`, wired
// in `dashboard.jsonnet`). The single panel meant to alarm is `invariant`,
// at the bottom.

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
  // RATE LIMITED — #230's own decision counter, the direct answer to
  // main.rs's former `TODO: Is this even working! boyo needs to know!`.
  rateLimited: {
    title: 'RATE LIMITED',
    type: 'stat',
    targets: [{ expr: 'sum(rate(rate_limit_decisions_total{outcome="rejected"}[5m]))', instant: true, refId: 'A' }],
    fieldConfig: statFieldConfig('reqps'),
    options: statOptions,
  },

  // SHED — #223's `load_shed` reason, the same series HEALTH's REFUSALS
  // stat sums across all reasons; this one isolates the load-shedding half.
  shed: {
    title: 'SHED',
    type: 'stat',
    targets: [{ expr: 'sum(rate(refusals_total{reason="load_shed"}[5m]))', instant: true, refId: 'A' }],
    fieldConfig: statFieldConfig('reqps'),
    options: statOptions,
  },

  // TIMED OUT — #223's `timeout` reason.
  timedOut: {
    title: 'TIMED OUT',
    type: 'stat',
    targets: [{ expr: 'sum(rate(refusals_total{reason="timeout"}[5m]))', instant: true, refId: 'A' }],
    fieldConfig: statFieldConfig('reqps'),
    options: statOptions,
  },

  // WS REFUSED — all three WebSocket admission refusals from #223
  // (`global_capacity`, `queue_full`, `permit_timeout`) summed by stage
  // rather than split three ways, since this row answers *whether* WS
  // admission is refusing work; `refusalsByReason` below answers *which*.
  wsRefused: {
    title: 'WS REFUSED',
    type: 'stat',
    targets: [{ expr: 'sum(rate(refusals_total{stage="ws"}[5m]))', instant: true, refId: 'A' }],
    fieldConfig: statFieldConfig('reqps'),
    options: statOptions,
  },

  // Refusals/sec by reason — every row of #223's refusal table, one line
  // each, `stage` included in the legend since `timeout`/`load_shed` are
  // HTTP-only and the three WS reasons don't collide with them but read
  // clearer labelled anyway.
  refusalsByReason: {
    title: 'Refusals/sec by Reason',
    type: 'timeseries',
    targets: [{ expr: 'sum by (stage, reason) (rate(refusals_total[5m]))', legendFormat: '{{stage}}/{{reason}}', refId: 'A' }],
    fieldConfig: timeSeriesFieldConfig { defaults+: { unit: 'reqps', custom+: { stacking: { mode: 'normal' } } } },
    options: timeSeriesOptions,
  },

  // Limiter: tokens available & tracked clients — #230's "useful for
  // debugging" gauge, now reachable. `rate_limit_tokens_available` is the
  // average remaining capacity across active per-client buckets (#231
  // partitioned the single global bucket this used to be); watching it
  // recover after a burst is the acceptance criterion #230 names.
  // `rate_limit_active_clients` rides the same panel as the visible half of
  // #231's "does not grow without bound" criterion — churn shows as this
  // line climbing and falling with traffic, not climbing forever.
  tokensAvailable: {
    title: 'Limiter: Tokens Available & Tracked Clients',
    type: 'timeseries',
    targets: [
      { expr: 'rate_limit_tokens_available', legendFormat: 'avg tokens available', refId: 'A' },
      { expr: 'rate_limit_active_clients', legendFormat: 'tracked clients', refId: 'B' },
    ],
    fieldConfig: timeSeriesFieldConfig { defaults+: { unit: 'none' } },
    options: timeSeriesOptions,
  },

  // INVARIANT — the payoff. Traffic above the *configured* limit
  // (`rate_limit_capacity_per_minute`, stamped once at startup from
  // `config.rate_limit` — see `metrics::rate_limit::record_capacity`) with
  // zero rejections in the same window is a contradiction: either the
  // limiter isn't running or it isn't enforcing. Deriving the threshold from
  // the live gauge instead of a hardcoded number (the parked version's `>
  // 20`) means this stays true if the limit is ever retuned — #232's own
  // task.
  //
  // The parked version combined its two conditions with a bare `and`, which
  // in PromQL is a *filter* — when the traffic side doesn't clear the
  // threshold (the common case: idle or under-limit), the whole expression
  // returns no series at all, indistinguishable from the metrics themselves
  // being absent. That ambiguity is exactly what #219 (G3) exists to rule
  // out. `> bool` / `== bool` instead force each side to always evaluate to
  // a definite 0 or 1, and multiplying them is a genuine logical AND with a
  // value on both branches — 1 only when both conditions truly hold, 0 when
  // either doesn't, and *empty* only when a source metric is genuinely
  // missing (correctly falling through to `panelDefaults.hardenAll`'s grey
  // "no data", applied in `dashboard.jsonnet`).
  invariant: {
    title: 'INVARIANT: Traffic Above Limit ∧ Zero Rejections',
    type: 'stat',
    targets: [{
      expr: |||
        (
          sum(rate(http_requests_total[1m])) > bool (rate_limit_capacity_per_minute / 60)
        )
        *
        (
          sum(rate(rate_limit_decisions_total{outcome="rejected"}[1m])) == bool 0
        )
      |||,
      instant: true,
      refId: 'A',
    }],
    fieldConfig: {
      defaults: {
        unit: 'none',
        mappings: [
          { type: 'value', options: {
            '0': { text: 'healthy', color: 'green' },
            '1': { text: 'FIRING — limiter not enforcing', color: 'red' },
          } },
        ],
        color: { mode: 'thresholds' },
        thresholds: { mode: 'absolute', steps: [{ color: panelDefaults.unknownColor, value: null }] },
      },
      overrides: [],
    },
    options: {
      colorMode: 'value',
      graphMode: 'none',
      justifyMode: 'auto',
      orientation: 'auto',
      reduceOptions: { calcs: ['lastNotNull'], values: false },
      textMode: 'auto',
    },
  },
}
