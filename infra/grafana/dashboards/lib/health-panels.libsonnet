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
        or (label_replace(vector(-2), "state", "no data", "__name__", ".*") unless on() count(dependency_up))
      |||,
      legendFormat: '{{state}}',
      instant: true,
      refId: 'A',
    }],
    fieldConfig: {
      defaults: {
        unit: 'none',
        color: { mode: 'thresholds' },
        // -2 is the "no data" row above: grey, never red or green.
        thresholds: { mode: 'absolute', steps: [{ color: panelDefaults.unknownColor, value: null }, { color: 'red', value: 0 }, { color: 'green', value: 1 }] },
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
    // A range query, not instant, so the tile carries its own sparkline:
    // "is it rising" is the second question a red ERRORS raises.
    targets: [{ expr: 'sum(rate(http_requests_total{status=~"5..", route!="/ready"}[5m])) or (0 * sum(rate(http_requests_total[5m])))', refId: 'A' }],
    fieldConfig: {
      defaults: {
        unit: 'reqps',
        decimals: 2,
        color: { mode: 'thresholds' },
        thresholds: { mode: 'absolute', steps: [{ color: 'green', value: null }, { color: 'red', value: 0.001 }] },
      },
      overrides: [],
    },
    options: { colorMode: 'value', graphMode: 'area', justifyMode: 'center', orientation: 'horizontal', reduceOptions: { calcs: ['lastNotNull'], values: false }, textMode: 'value' },
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
    targets: [{ expr: 'sum(rate(refusals_total[5m]))', refId: 'A' }],
    fieldConfig: {
      defaults: {
        unit: 'ops',
        decimals: 2,
        color: { mode: 'thresholds' },
        thresholds: { mode: 'absolute', steps: [{ color: 'green', value: null }, { color: 'red', value: 0.001 }] },
      },
      overrides: [],
    },
    options: { colorMode: 'value', graphMode: 'area', justifyMode: 'center', orientation: 'horizontal', reduceOptions: { calcs: ['lastNotNull'], values: false }, textMode: 'value' },
  },

  // LOOPS — condition #5 (stalled). Three states, most severe first:
  // STALLED (no pass has *started* in three configured intervals — the task
  // died or a pass is hung), FAILING · <class> (passes run, and the last one
  // returned an error — the class names what to fix, e.g. `schema` for a
  // database missing migrations), ok. Deriving the threshold from the
  // exported interval keeps a tuned deployment honest. Below those, three
  // states that aren't findings about the loop: NOT RUNNING (enabled, but no
  // pass has ever started), off (`NUDGE_ENABLED` is false — the default — so
  // there is no loop to judge; before `nudge_waker_enabled` existed this
  // rendered blank, and SIGNAL called it BLIND), and no data. `textMode:
  // 'name'` hides Grafana's own "no data" placeholder, which is why that one
  // is an explicit row rather than left to `harden()`. The stall predicate
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
        or label_replace((max(nudge_waker_enabled) == 1) unless on() max(nudge_waker_last_attempt_timestamp_seconds), "state", "NOT RUNNING", "__name__", ".*")
        or label_replace((max(nudge_waker_enabled) == 0) - 1, "state", "off", "__name__", ".*")
        or (label_replace(vector(-2), "state", "no data", "__name__", ".*") unless on() max(nudge_waker_enabled))
      |||,
      legendFormat: '{{state}}',
      instant: true,
      refId: 'A',
    }],
    fieldConfig: {
      defaults: {
        unit: 'none',
        color: { mode: 'thresholds' },
        // -2 no data (grey) / -1 off (blue: the deployment chose not to
        // run the waker — `nudge_waker_enabled` 0, see metrics/waker.rs) /
        // 0 ok / 1 STALLED, FAILING or NOT RUNNING.
        thresholds: { mode: 'absolute', steps: [{ color: panelDefaults.unknownColor, value: null }, { color: 'blue', value: -1 }, { color: 'green', value: 0 }, { color: 'red', value: 1 }] },
      },
      overrides: [],
    },
    options: { colorMode: 'value', graphMode: 'none', justifyMode: 'center', orientation: 'horizontal', reduceOptions: { calcs: ['lastNotNull'], values: false }, textMode: 'name' },
  },

  // SIGNAL — condition #6 (blind). Red the instant any of the five panels
  // above has lost the series it reads, and — unlike the first version,
  // which only said BLIND — naming which one, so the tile says where to look
  // rather than only that something is wrong. One `absent()` term per
  // signal, each stamped with its name (`label_replace`, same idiom as DEPS)
  // and weighted by the HEALTH row's own left-to-right precedence; `topk(1)`
  // keeps only the most upstream blindness (a dead scrape makes every other
  // series absent too — naming it "deps" would point at the wrong fix). A
  // dead scrape is `up == 0`, not only an absent `up`: Prometheus keeps
  // writing `up{job="file_host"} 0` for an unreachable target, so `absent()`
  // alone never fired and the stale app series surfaced as "deps". The
  // `ok` row competes at 0, so it wins exactly when nothing is absent.
  //
  // The waker's three series only count when `nudge_waker_enabled` says the
  // waker runs: a deployment with nudges off (the default) exports none of
  // them, and reading that as blindness is how this tile sat at BLIND on a
  // perfectly healthy server. `nudge_waker_enabled` itself must exist either
  // way — main.rs sets it on both branches of the nudge gate.
  signal: {
    title: 'SIGNAL',
    type: 'stat',
    id: 905,
    links: docLink('blind'),
    targets: [{
      // Same reasoning as LOOPS above about writing the metric names out
      // literally rather than interpolating them.
      expr: |||
        topk(1,
          label_replace((absent(up{job="file_host"}) or on() ((max(up{job="file_host"}) == 0) + 1)) * 7, "state", "BLIND · scrape", "__name__", ".*")
          or label_replace(absent(dependency_up) * 6, "state", "BLIND · deps", "__name__", ".*")
          or label_replace(absent(http_requests_total) * 5, "state", "BLIND · http", "__name__", ".*")
          or label_replace(absent(refusals_total) * 4, "state", "BLIND · refusals", "__name__", ".*")
          or label_replace(absent(nudge_waker_enabled) * 3, "state", "BLIND · waker", "__name__", ".*")
          or label_replace(
            (absent(nudge_waker_last_attempt_timestamp_seconds) or absent(nudge_waker_pass_failing) or absent(nudge_waker_interval_seconds)) * 2
              and on() (max(nudge_waker_enabled) == 1),
            "state", "BLIND · loops", "__name__", ".*")
          or label_replace(vector(0), "state", "ok", "__name__", ".*")
        )
      |||,
      legendFormat: '{{state}}',
      instant: true,
      refId: 'A',
    }],
    fieldConfig: {
      defaults: {
        unit: 'none',
        color: { mode: 'thresholds' },
        thresholds: { mode: 'absolute', steps: [{ color: panelDefaults.unknownColor, value: null }, { color: 'green', value: 0 }, { color: 'red', value: 1 }] },
      },
      overrides: [],
    },
    options: { colorMode: 'value', graphMode: 'none', justifyMode: 'center', orientation: 'horizontal', reduceOptions: { calcs: ['lastNotNull'], values: false }, textMode: 'name' },
  },

  // #213/F4's other half — which build is this, and how long has it been
  // running — used to be two strips here (`buildInfo`, `uptime`) that spent
  // a full row on a line of 8pt text. The build is now the `build`/`built`
  // pickers in dashboard.jsonnet's header (from `service_info`'s labels),
  // and uptime is overview-panels.libsonnet's UP FOR tile.

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
