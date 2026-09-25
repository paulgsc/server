// glance.libsonnet
//
// Panel constructors shared by the two "at a glance" layers —
// overview-panels.libsonnet (file_host) and sympathy-panels.libsonnet (WHO
// DUNNIT) — so a verdict tile, a trend tile and a ranked bar read the same on
// both dashboards. See overview-panels.libsonnet's header for the design rules
// these encode.
local ds = { type: 'prometheus', uid: 'prometheus' };

local steps(list) = { mode: 'absolute', steps: list };
local green = 'green';
local amber = 'orange';
local red = 'red';

local sparkStat(title, description, expr, unit, thresholds, decimals=null) = {
  title: title,
  description: description,
  type: 'stat',
  datasource: ds,
  targets: [{ expr: expr, refId: 'A' }],
  fieldConfig: {
    defaults: {
      unit: unit,
      color: { mode: 'thresholds' },
      thresholds: thresholds,
    } + (if decimals != null then { decimals: decimals } else {}),
    overrides: [],
  },
  options: {
    colorMode: 'value',
    graphMode: 'area',
    justifyMode: 'center',
    orientation: 'auto',
    textMode: 'value',
    wideLayout: true,
    showPercentChange: false,
    reduceOptions: { calcs: ['lastNotNull'], values: false },
  },
};

// A figure over the whole selected time range (`$__range`) — one number, no
// sparkline, since a sparkline of a range aggregate is a rolling window that
// means something different from the number beside it.
local rangeStat(title, description, expr, unit, thresholds, decimals=null) =
  sparkStat(title, description, expr, unit, thresholds, decimals) {
    targets: [{ expr: expr, instant: true, refId: 'A' }],
    options+: { graphMode: 'none' },
  };

local informational = steps([{ color: 'blue', value: null }]);

local barGauge(title, description, targets, unit, thresholds, max=null) = {
  title: title,
  description: description,
  type: 'bargauge',
  datasource: ds,
  targets: targets,
  fieldConfig: {
    defaults: {
      unit: unit,
      min: 0,
      color: { mode: 'thresholds' },
      thresholds: thresholds,
    } + (if max != null then { max: max } else {}),
    overrides: [],
  },
  options: {
    displayMode: 'gradient',
    orientation: 'horizontal',
    showUnfilled: true,
    valueMode: 'color',
    // Names on their own line above each bar: beside it, Grafana truncates
    // "file-host-server" to "file-h…" at any usable panel width.
    namePlacement: 'top',
    sizing: 'auto',
    minVizHeight: 16,
    minVizWidth: 8,
    maxVizHeight: 300,
    text: { titleSize: 12, valueSize: 13 },
    reduceOptions: { calcs: ['lastNotNull'], values: false },
  },
};

{
  steps:: steps,
  green:: green,
  amber:: amber,
  red:: red,
  informational:: informational,
  sparkStat:: sparkStat,
  rangeStat:: rangeStat,
  barGauge:: barGauge,

  // A plain trend chart: lines, light fill, legend with last/max, shared
  // crosshair (the dashboards set graphTooltip: 1).
  trend(title, description, targets, unit, max=null):: {
    title: title,
    description: description,
    type: 'timeseries',
    datasource: ds,
    targets: targets,
    fieldConfig: {
      defaults: {
        unit: unit,
        min: 0,
        color: { mode: 'palette-classic' },
        custom: { drawStyle: 'line', lineWidth: 2, fillOpacity: 12, gradientMode: 'opacity', showPoints: 'never', spanNulls: false, stacking: { mode: 'none' } },
      } + (if max != null then { max: max } else {}),
      overrides: [],
    },
    options: {
      legend: { showLegend: true, displayMode: 'table', placement: 'right', calcs: ['lastNotNull', 'max'] },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
  },
}
