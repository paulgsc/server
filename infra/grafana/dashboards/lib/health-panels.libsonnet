// health-panels.libsonnet
//
// The HEALTH row — #213/#225 (F5). Six conditions, six stat panels, all
// boring: green/red/grey and nothing else. See docs/fault-conditions.md for
// the strict definition each panel renders a verdict on; each panel below
// links back to its own condition's anchor there.
local panelDefaults = import 'panel-defaults.libsonnet';

local docsUrl = 'https://github.com/paulgsc/server/blob/main/docs/fault-conditions.md';

// #216/P4 (#236) exports the live configured interval, when the most recent
// pass started, and whether the last one that finished failed (and in which
// class — see metrics/waker.rs). An empty pass counts as successful: quiet
// success and a stopped task are precisely the two states this signal must
// separate, and a failing pass is a third that must not read as either.

// A verdict panel says what is wrong, not only that something is: DEPS and
// LOOPS each return exactly one series per finding, carrying the finding in
// a `state` label, and render it with `textMode: 'name'` (the same
// mechanism `buildInfo` below uses for its labels). The value only picks the
// color. `label_replace(x, "state", "…", "__name__", ".*")` is the idiom
// for stamping a constant label onto an aggregated series (which has no
// `__name__`, so `.*` matches the empty string); `A or (B unless on() A)`
// is precedence, so only the most severe finding renders. Checked with
// `promtool test rules` (Prometheus 2.54) when written — each state, the
// precedence, and no-data when the series are absent — but nothing in CI
// re-checks PromQL semantics, so re-run that by hand after changing these.

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

  // DEPS — condition #2 (dependency down). One red tile per dependency
  // that is down, named ("schema down" rather than a bare "3/4"), or a
  // single green "all up". `schema` (#fault-conditions) is the one that
  // isn't a connection: migrations this build expects that the database
  // hasn't applied — `/ready`'s body lists which. Hidden while `sqlite`
  // itself is down: an unreadable database can't have its migrations read
  // either, so the schema tile would be the same cause a second time — and
  // would point at `sqlx migrate run` for what is really a missing volume.
  deps: {
    title: 'DEPS',
    type: 'stat',
    id: 901,
    links: docLink('dependency-down'),
    targets: [{
      expr: |||
        label_replace(
          max by (dependency) (dependency_up == 0)
            unless on(dependency) (max by (dependency) (dependency_up{dependency="schema"} == 0) and on() max(dependency_up{dependency="sqlite"} == 0)),
          "state", "$1 down", "dependency", "(.*)"
        )
        or (label_replace(0 * count(dependency_up) + 1, "state", "all up", "__name__", ".*") unless on() max(dependency_up == 0))
      |||,
      legendFormat: '{{state}}',
      instant: true,
      refId: 'A',
    }],
    fieldConfig: {
      defaults: {
        unit: 'none',
        color: { mode: 'thresholds' },
        thresholds: { mode: 'absolute', steps: [{ color: 'red', value: null }, { color: 'green', value: 1 }] },
      },
      overrides: [],
    },
    options: { colorMode: 'value', graphMode: 'none', justifyMode: 'center', orientation: 'horizontal', reduceOptions: { calcs: ['lastNotNull'], values: false }, textMode: 'name' },
  },

  // ERRORS — condition #3 (rejecting). Sustained non-zero 5xx rate is red;
  // the 5m rate window is what makes "sustained" mean something rather than
  // reddening on a single blip. `/ready` is excluded: its 503 *is* DEPS's
  // finding, answered to the container healthcheck every 30s, and counting
  // it here turned one down dependency into two red panels. The `or 0 *`
  // fallback reads 0 (green) when no other route has ever produced a 5xx —
  // `sum()` of an empty selector is no data, not zero, which would trade
  // the duplicate red for grey — gated on `http_requests_total` existing
  // at all, so a genuinely missing family still reads no-data for SIGNAL.
  errors: {
    title: 'ERRORS',
    type: 'stat',
    id: 902,
    links: docLink('rejecting'),
    targets: [{ expr: 'sum(rate(http_requests_total{status=~"5..", route!="/ready"}[5m])) or (0 * sum(rate(http_requests_total[5m])))', instant: true, refId: 'A' }],
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

  // LOOPS — condition #5 (stalled). Three states, most severe first:
  // STALLED (no pass has *started* in three configured intervals — the task
  // died or a pass is hung), FAILING · <class> (passes run, and the last one
  // returned an error — the class names what to fix, e.g. `schema` for a
  // database missing migrations), ok. Deriving the threshold from the
  // exported interval keeps a tuned deployment honest. The stall predicate
  // is repeated rather than factored into a jsonnet local on purpose:
  // scripts/check_metric_contract.py reads metric names lexically out of
  // `expr:` strings, and an interpolated name is one it can't see.
  loops: {
    title: 'LOOPS',
    type: 'stat',
    id: 904,
    links: docLink('stalled'),
    targets: [{
      expr: |||
        label_replace((time() - max(nudge_waker_last_attempt_timestamp_seconds)) > 3 * max(nudge_waker_interval_seconds), "state", "STALLED", "__name__", ".*")
        or (label_replace(max by (error) (nudge_waker_pass_failing == 1), "state", "FAILING · $1", "error", "(.*)") unless on() ((time() - max(nudge_waker_last_attempt_timestamp_seconds)) > 3 * max(nudge_waker_interval_seconds)))
        or (label_replace(0 * max(nudge_waker_last_attempt_timestamp_seconds), "state", "ok", "__name__", ".*") unless on() (((time() - max(nudge_waker_last_attempt_timestamp_seconds)) > 3 * max(nudge_waker_interval_seconds)) or max(nudge_waker_pass_failing == 1)))
      |||,
      legendFormat: '{{state}}',
      instant: true,
      refId: 'A',
    }],
    fieldConfig: {
      defaults: {
        unit: 'none',
        color: { mode: 'thresholds' },
        thresholds: { mode: 'absolute', steps: [{ color: 'green', value: null }, { color: 'red', value: 1 }] },
      },
      overrides: [],
    },
    options: { colorMode: 'value', graphMode: 'none', justifyMode: 'center', orientation: 'horizontal', reduceOptions: { calcs: ['lastNotNull'], values: false }, textMode: 'name' },
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
      // Same reasoning as LOOPS above about writing the metric names out
      // literally rather than interpolating them.
      expr: |||
        (
          sum(absent(up{job="file_host"}))
          or sum(absent(dependency_up))
          or sum(absent(http_requests_total))
          or sum(absent(refusals_total))
          or sum(absent(nudge_waker_last_attempt_timestamp_seconds))
          or sum(absent(nudge_waker_pass_failing))
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
    targets: [{ expr: 'time() - process_start_time_seconds{job="file_host"}', instant: true, refId: 'A' }],
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
