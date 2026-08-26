// nudge-panels.libsonnet
//
// The NUDGE row. LOOPS (health-panels.libsonnet, condition #5/stalled) only
// proves the engagement waker's tick is alive — a pass that runs on
// schedule and finds every due subject suppressed, or claims one and then
// has every device reject the push, leaves LOOPS exactly as green as a
// quiet day with nothing due. Both are "nudge_waker_last_pass_timestamp_seconds
// moved recently"; neither tells you which one happened. That gap is what
// let #257's cold-start silence — the nudge saying nothing for exactly the
// subject who most needed it — read as "nudge: ok" on this dashboard for as
// long as it went unnoticed.
//
// DUE and the outcomes breakdown below close it, on `nudge_waker_verdicts_total`
// and `nudge_waker_due_subjects` (nudge/waker.rs, metrics/waker.rs). This is
// deliberately a breakdown, not a red/green invariant like ABUSE's: a
// subject correctly suppressed by quiet hours, or by not having consented
// to this topic, is not a fault, and #212's own non-goal ("no alerting —
// the human is the alertmanager") applies here as much as anywhere else in
// this dashboard. What a human needs is to see *which* outcome is
// happening, sustained, when DUE stays above zero and nothing is landing —
// suppressed (and by which reason), stuck on nothing-to-say, or claimed and
// then rejected by every device (the VAPID-mismatch/pruned-subscription
// failure `docs/study-nudge.md` calls the single longest-fuse way this
// feature goes quiet).

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

{
  // DUE — how many subjects the waker's last pass found past `eligible_at`.
  // Read next to the breakdown below, not against its own threshold: DUE
  // climbing while the breakdown shows no `sent` outcomes is the pattern
  // worth noticing, not any particular value of DUE itself — see #216/P4's
  // own point that a quiet day is supposed to look like zero.
  due: {
    title: 'DUE',
    type: 'stat',
    targets: [{ expr: 'nudge_waker_due_subjects', instant: true, refId: 'A' }],
    fieldConfig: statFieldConfig('none'),
    options: statOptions,
  },

  // Fixed-label operational configuration failures since this process
  // started — the narrow signal the NUDGE_TIMEZONE incident asked for,
  // kept on this row so an operator watching nudge specifically doesn't
  // have to find it in the workspace-wide by-target breakdown
  // (panels.libsonnet's tracingErrors, below the ABUSE row). `up == 1`
  // guard: an absent scrape target is condition #1 (unreachable)'s job to
  // report, not this panel's — see docs/dashboard-honesty.md.
  configErrors: {
    title: 'Nudge Config Errors',
    description: 'Startup configuration faults (e.g. an unparseable NUDGE_TIMEZONE) since the current file_host process started.',
    type: 'stat',
    targets: [{
      expr: '(sum(operation_errors_total{operation="nudge_config"}) or vector(0)) and on() (up{job="file_host"} == 1)',
      instant: true,
      refId: 'A',
    }],
    fieldConfig: {
      defaults: {
        unit: 'short',
        color: { mode: 'thresholds' },
        thresholds: { mode: 'absolute', steps: [{ color: 'green', value: null }, { color: 'red', value: 1 }] },
        noValue: 'no data',
      },
      overrides: [],
    },
    options: statOptions,
  },

  // Nudge outcomes by verdict/reason — every due subject's terminal branch
  // this pass, stacked. `sent` is the only outcome that put a notification
  // on the wire; everything else is a reason nothing went out, named rather
  // than left to be inferred from DUE staying nonzero with silence. Reading
  // this alongside DUE is the whole point: DUE > 0 with the stack showing
  // only `suppressed/quiet-hours` is correct behaviour, DUE > 0 with the
  // stack showing only `no_device_accepted` is the VAPID-rotation failure
  // mode, and DUE > 0 with an empty stack under it would itself be a gap
  // worth investigating (a verdict this dashboard doesn't yet know about).
  outcomesByVerdict: {
    title: 'Nudge Outcomes by Verdict',
    type: 'timeseries',
    targets: [{ expr: 'sum by (verdict, reason) (rate(nudge_waker_verdicts_total[5m]))', legendFormat: '{{verdict}} / {{reason}}', refId: 'A' }],
    fieldConfig: {
      defaults: {
        unit: 'ops',
        custom: { drawStyle: 'line', fillOpacity: 15, lineWidth: 2, pointSize: 4, showPoints: 'never', spanNulls: false, stacking: { mode: 'normal' } },
        color: { mode: 'palette-classic' },
      },
      overrides: [
        { matcher: { id: 'byName', options: 'sent / n/a' }, properties: [{ id: 'color', value: { mode: 'fixed', fixedColor: 'green' } }] },
        { matcher: { id: 'byName', options: 'no_device_accepted / n/a' }, properties: [{ id: 'color', value: { mode: 'fixed', fixedColor: 'red' } }] },
      ],
    },
    options: { legend: { showLegend: true, placement: 'bottom' }, tooltip: { mode: 'multi', sort: 'desc' } },
  },
}
