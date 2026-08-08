// panel-defaults.libsonnet
//
// See #219 (G3). Grafana's own defaults make "no data" indistinguishable
// from "measured and fine": a stat/gauge panel reducing over zero series
// still produces a null value, and every threshold list in this repo starts
// `{ value: null, color: 'green' }` — so an absent metric renders exactly
// like a healthy one. Three states, not two, and they must be
// distinguishable at a glance:
//
//   data, in bounds      -> green
//   data, out of bounds  -> red
//   no data              -> grey, never green
//
// `harden` is what actually produces the third state: it merges a "null or
// NaN" value mapping into a stat/gauge panel's fieldConfig, which Grafana
// resolves before falling back to threshold-based coloring. Time series
// panels are left alone — a gap in a line is already self-evident and
// `spanNulls: false` (already the default across this repo) is enough.
{
  // Grafana's own "text-secondary" grey. Never used by a real threshold
  // step anywhere in this repo, so it reads as "not a measurement" on sight.
  unknownColor: '#8E8E8E',

  noDataMapping:: {
    type: 'special',
    options: {
      match: 'null+nan',
      result: { text: 'no data', color: $.unknownColor },
    },
  },

  // Mix into a stat/gauge panel to make its empty-result state render grey
  // instead of falling through to the base (green) threshold step. Safe to
  // call on any panel — rows and time series pass through unchanged.
  harden(panel)::
    if panel.type == 'stat' || panel.type == 'gauge' then
      panel {
        fieldConfig+: {
          defaults+: {
            noValue: 'no data',
            mappings+: [$.noDataMapping],
          },
        },
      }
    else
      panel,

  // Apply `harden` to an entire dashboard's panel list in one call:
  // `panels: panelDefaults.hardenAll([...])`.
  hardenAll(panels):: std.map($.harden, panels),

  // One `up{job="..."}` liveness stat per scraped job a dashboard depends
  // on, so "no data everywhere" reads as a dead scrape target rather than a
  // suspiciously quiet system. One panel, one target per job — 0/1 mapped
  // to DOWN/UP directly (not through thresholds), no-data still grey.
  livenessPanel(title, jobs, id, gridPos):: {
    title: title,
    type: 'stat',
    id: id,
    gridPos: gridPos,
    datasource: { type: 'prometheus', uid: 'prometheus' },
    fieldConfig: {
      defaults: {
        color: { mode: 'thresholds' },
        mappings: [
          {
            type: 'value',
            options: {
              '0': { text: 'DOWN', color: 'red' },
              '1': { text: 'UP', color: 'green' },
            },
          },
          $.noDataMapping,
        ],
        thresholds: { mode: 'absolute', steps: [{ color: $.unknownColor, value: null }] },
        unit: 'none',
        noValue: 'no data',
      },
      overrides: [],
    },
    options: {
      colorMode: 'value',
      graphMode: 'none',
      justifyMode: 'center',
      orientation: 'horizontal',
      reduceOptions: { calcs: ['lastNotNull'], values: false },
      textMode: 'name_and_value',
    },
    targets: std.mapWithIndex(
      function(i, job) {
        expr: 'up{job="%s"}' % job,
        legendFormat: job,
        refId: std.char(65 + i),
        instant: true,
      },
      jobs
    ),
  },
}
