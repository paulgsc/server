// health-panels.libsonnet
//
// The HEALTH row — #213/#225 (F5). Six conditions, six stat panels, all
// boring: green/red/grey and nothing else. See docs/fault-conditions.md for
// the strict definition each panel renders a verdict on; each panel below
// links back to its own condition's anchor there.
local panelDefaults = import 'panel-defaults.libsonnet';

local docsUrl = 'https://github.com/paulgsc/server/blob/main/docs/fault-conditions.md';

// #216/P4 (#236) exports both the most recent successful pass and the live
// configured interval. An empty pass counts as successful: quiet success and
// a stopped task are precisely the two states this signal must separate.

local docLink(anchor) = [
  {
    title: 'What this means',
    url: docsUrl + '#' + anchor,
    targetBlank: true,
  },
];

{
  // UP — condition #1 (unreachable). Reuses panelDefaults.livenessPanel, the
  // same 0/1/no-data convention used by every other dashboard's liveness row.
  up: panelDefaults.livenessPanel('UP', ['file_host'], 900, {}) { links: docLink('unreachable') },

  // DEPS — condition #2 (dependency down). `unit: '/3'` is Grafana's plain
  // custom-suffix mechanism — a raw value of 3 renders as "3/3".
  deps: {
    title: 'DEPS',
    type: 'stat',
    id: 901,
    links: docLink('dependency-down'),
    targets: [{ expr: 'sum(dependency_up)', instant: true, refId: 'A' }],
    fieldConfig: {
      defaults: {
        unit: '/3',
        color: { mode: 'thresholds' },
        thresholds: { mode: 'absolute', steps: [{ color: 'red', value: null }, { color: 'green', value: 3 }] },
      },
      overrides: [],
    },
    options: { colorMode: 'value', graphMode: 'none', justifyMode: 'center', orientation: 'horizontal', reduceOptions: { calcs: ['lastNotNull'], values: false }, textMode: 'value' },
  },

  // ERRORS — condition #3 (rejecting). Sustained non-zero 5xx rate is red;
  // the 5m rate window is what makes "sustained" mean something rather than
  // reddening on a single blip.
  errors: {
    title: 'ERRORS',
    type: 'stat',
    id: 902,
    links: docLink('rejecting'),
    targets: [{ expr: 'sum(rate(http_requests_total{status=~"5.."}[5m]))', instant: true, refId: 'A' }],
    fieldConfig: {
      defaults: {
        unit: 'reqps',
        decimals: 2,
        color: { mode: 'thresholds' },
        thresholds: { mode: 'absolute', steps: [{ color: 'green', value: null }, { color: 'red', value: 0.001 }] },
      },
      overrides: [],
    },
    options: { colorMode: 'value', graphMode: 'none', justifyMode: 'center', orientation: 'horizontal', reduceOptions: { calcs: ['lastNotNull'], values: false }, textMode: 'value' },
  },

  // REFUSALS — condition #4 (saturated). Same "sustained non-zero is red"
  // shape as ERRORS, over every reason #223 counts (http and ws stages
  // alike — this panel answers *whether*, not *which*; the request-rate
  // timeseries below and refusals_total itself answer *which*).
  refusals: {
    title: 'REFUSALS',
    type: 'stat',
    id: 903,
    links: docLink('saturated'),
    targets: [{ expr: 'sum(rate(refusals_total[5m]))', instant: true, refId: 'A' }],
    fieldConfig: {
      defaults: {
        unit: 'ops',
        decimals: 2,
        color: { mode: 'thresholds' },
        thresholds: { mode: 'absolute', steps: [{ color: 'green', value: null }, { color: 'red', value: 0.001 }] },
      },
      overrides: [],
    },
    options: { colorMode: 'value', graphMode: 'none', justifyMode: 'center', orientation: 'horizontal', reduceOptions: { calcs: ['lastNotNull'], values: false }, textMode: 'value' },
  },

  // LOOPS — condition #5 (stalled). Three missed configured intervals is a
  // fault; deriving the threshold from the exported interval keeps a tuned
  // deployment honest.
  loops: {
    title: 'LOOPS',
    type: 'stat',
    id: 904,
    links: docLink('stalled'),
    targets: [{ expr: '(time() - nudge_waker_last_pass_timestamp_seconds) > bool (3 * nudge_waker_interval_seconds)', instant: true, refId: 'A' }],
    fieldConfig: {
      defaults: {
        unit: 'none',
        mappings: [{ type: 'value', options: { '0': { text: 'ok', color: 'green' }, '1': { text: 'STALLED', color: 'red' } } }],
        color: { mode: 'thresholds' },
        thresholds: { mode: 'absolute', steps: [{ color: panelDefaults.unknownColor, value: null }] },
      },
      overrides: [],
    },
    options: { colorMode: 'value', graphMode: 'none', justifyMode: 'center', orientation: 'horizontal', reduceOptions: { calcs: ['lastNotNull'], values: false }, textMode: 'value' },
  },

  // SIGNAL — condition #6 (blind). Red the instant *any* of the five panels
  // above is grey, so "five greens" and "five greys" are never mistaken for
  // each other at a glance. `sum(absent(x))` collapses every absent() call
  // to the same unlabeled series regardless of x's own label selector
  // (Prometheus's `absent()` otherwise propagates equality-matched labels
  // from the selector, which would keep e.g. `up{job="file_host"}`'s check
  // from `or`-combining cleanly with the label-free ones); `or vector(0)`
  // is the standard idiom for "1 if anything on the left fired, else 0" —
  // reasoned through, not verified against a live Prometheus.
  signal: {
    title: 'SIGNAL',
    type: 'stat',
    id: 905,
    links: docLink('blind'),
    targets: [{
      // Same reasoning as LOOPS above about writing the metric name out
      // literally rather than interpolating `waker_last_pass`.
      expr: |||
        (
          sum(absent(up{job="file_host"}))
          or sum(absent(dependency_up))
          or sum(absent(http_requests_total))
          or sum(absent(refusals_total))
          or sum(absent(nudge_waker_last_pass_timestamp_seconds))
          or sum(absent(nudge_waker_interval_seconds))
        ) or vector(0)
      |||,
      instant: true,
      refId: 'A',
    }],
    fieldConfig: {
      defaults: {
        unit: 'none',
        mappings: [{ type: 'value', options: { '0': { text: 'ok', color: 'green' }, '1': { text: 'BLIND', color: 'red' } } }],
        color: { mode: 'thresholds' },
        thresholds: { mode: 'absolute', steps: [{ color: panelDefaults.unknownColor, value: null }] },
      },
      overrides: [],
    },
    options: { colorMode: 'value', graphMode: 'none', justifyMode: 'center', orientation: 'horizontal', reduceOptions: { calcs: ['lastNotNull'], values: false }, textMode: 'value' },
  },

  // #213/F4's other half: which build is this, and how long has it been
  // running. Not one of the six conditions, but the context that makes
  // reading any of them trustworthy — "how long has it been like this" and
  // "is this the build I think it is" are the first two questions a red
  // panel raises. `service_info` is a fixed-1 info gauge; `textMode: 'name'`
  // renders its labels (via legendFormat) instead of the value, which is
  // the conventional way to surface an info-metric's labels as text.
  buildInfo: {
    title: 'Build',
    type: 'stat',
    id: 906,
    targets: [{ expr: 'service_info', legendFormat: '{{git_sha}} · rustc {{rust}} · built {{built_at}}', instant: true, refId: 'A' }],
    fieldConfig: {
      defaults: {
        unit: 'none',
        color: { mode: 'fixed', fixedColor: 'text' },
        thresholds: { mode: 'absolute', steps: [{ color: panelDefaults.unknownColor, value: null }] },
      },
      overrides: [],
    },
    options: { colorMode: 'none', graphMode: 'none', justifyMode: 'center', orientation: 'horizontal', reduceOptions: { calcs: ['lastNotNull'], values: false }, textMode: 'name' },
  },

  uptime: {
    title: 'Uptime',
    type: 'stat',
    id: 907,
    targets: [{ expr: 'time() - process_start_time_seconds', instant: true, refId: 'A' }],
    fieldConfig: {
      defaults: {
        unit: 's',
        color: { mode: 'fixed', fixedColor: 'text' },
        thresholds: { mode: 'absolute', steps: [{ color: panelDefaults.unknownColor, value: null }] },
      },
      overrides: [],
    },
    options: { colorMode: 'none', graphMode: 'none', justifyMode: 'center', orientation: 'horizontal', reduceOptions: { calcs: ['lastNotNull'], values: false }, textMode: 'value' },
  },

  // The wide timeseries below the stat row — "the shape of the day in one
  // glance". Outcome-split: served (2xx/3xx), 4xx, 5xx, refused.
  requestRateByOutcome: {
    title: 'Request Rate by Outcome',
    type: 'timeseries',
    id: 908,
    targets: [
      { expr: 'sum(rate(http_requests_total{status=~"2..|3.."}[5m]))', legendFormat: 'served', refId: 'A' },
      { expr: 'sum(rate(http_requests_total{status=~"4.."}[5m]))', legendFormat: '4xx', refId: 'B' },
      { expr: 'sum(rate(http_requests_total{status=~"5.."}[5m]))', legendFormat: '5xx', refId: 'C' },
      { expr: 'sum(rate(refusals_total[5m]))', legendFormat: 'refused', refId: 'D' },
    ],
    fieldConfig: {
      defaults: {
        unit: 'reqps',
        custom: { drawStyle: 'line', fillOpacity: 15, lineWidth: 2, pointSize: 4, showPoints: 'never', spanNulls: false, stacking: { mode: 'normal' } },
        color: { mode: 'palette-classic' },
      },
      overrides: [
        { matcher: { id: 'byName', options: '5xx' }, properties: [{ id: 'color', value: { mode: 'fixed', fixedColor: 'red' } }] },
        { matcher: { id: 'byName', options: 'refused' }, properties: [{ id: 'color', value: { mode: 'fixed', fixedColor: 'orange' } }] },
        { matcher: { id: 'byName', options: 'served' }, properties: [{ id: 'color', value: { mode: 'fixed', fixedColor: 'green' } }] },
      ],
    },
    options: { legend: { showLegend: true, placement: 'bottom' }, tooltip: { mode: 'multi', sort: 'desc' } },
  },

  // #213/F2's duration histogram, dashboarded — per-route latency across
  // the whole HTTP surface, distinct from `operationDuration` below (which
  // is `operation_duration_seconds`, a narrower per-operation/phase
  // histogram from `metrics::instruments`). Buckets are
  // `metrics::http::HTTP_DURATION_BUCKETS` — the top one sits above the
  // 15s request timeout on purpose, so a request that times out still
  // lands in a real bucket instead of falling off into `+Inf`.
  httpLatencyByRoute: {
    title: 'HTTP Latency by Route (P95)',
    type: 'timeseries',
    id: 909,
    targets: [{ expr: 'histogram_quantile(0.95, sum(rate(http_request_duration_seconds_bucket[5m])) by (le, route))', legendFormat: '{{route}}', refId: 'A' }],
    fieldConfig: {
      defaults: {
        unit: 's',
        custom: { drawStyle: 'line', fillOpacity: 10, lineWidth: 2, pointSize: 4, showPoints: 'never', spanNulls: false, stacking: { mode: 'none' } },
        color: { mode: 'palette-classic' },
      },
      overrides: [],
    },
    options: { legend: { showLegend: true, placement: 'right', calcs: ['last', 'max'] }, tooltip: { mode: 'multi', sort: 'desc' } },
  },
}
