// upgrade-panels.libsonnet
//
// The UPGRADES dashboard's panels: the box's nightly NixOS + home-manager
// upgrade, as judged by paulgsc/dotfiles nixos/auto-upgrade/report.sh. That
// script writes /var/lib/upgrade-report/metrics/upgrade.prom after every run,
// and node_exporter's textfile collector serves it
// (infra/compose/monitoring.yml), so every `upgrade_*` series is a gauge set
// once a night, not a counter.
//
// A run is GOOD only if both units succeeded, the system and home profile are
// both the commit `main` points at, and the capture's canary came back. The
// healthy state is quiet: one GOOD a night, nothing in FINDINGS. So "quiet"
// must not read like "not measured": METRICS FILE says whether node_exporter
// could read the file at all, SINCE LAST RUN whether the nightly run is still
// happening, and no data stays grey (panel-defaults).
//
// Rolling rates come precomputed from the box's own history file: Prometheus
// keeps ~8 days here (--storage.tsdb.retention.time=200h), too short for a
// 90-day window. The off-box judge of the same SLO is dotfiles'
// .github/workflows/upgrade-slo.yml, which opens an `slo` issue; this
// dashboard is for looking, that workflow is for being told.

local ds = { type: 'prometheus', uid: 'prometheus' };
local unknown = '#8E8E8E';
local repo = 'https://github.com/paulgsc/dotfiles';

local yesNo(no='NO', yes='YES') = [
  { type: 'value', options: { '0': { text: no, color: 'red' }, '1': { text: yes, color: 'green' } } },
];

local stat(title, description, targets, unit, steps, mappings=[], textMode='value') = {
  title: title,
  description: description,
  type: 'stat',
  datasource: ds,
  targets: [t { instant: true } for t in targets],
  fieldConfig: {
    defaults: {
      unit: unit,
      color: { mode: 'thresholds' },
      thresholds: { mode: 'absolute', steps: steps },
      mappings: mappings,
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
    textMode: textMode,
  },
};

local table(title, description, expr, exclude, rename, overrides=[], sortBy=[]) = {
  title: title,
  description: description,
  type: 'table',
  datasource: ds,
  targets: [{ expr: expr, format: 'table', instant: true, refId: 'A' }],
  fieldConfig: {
    defaults: { custom: { align: 'left', cellOptions: { type: 'auto' } } },
    overrides: overrides,
  },
  options: { showHeader: true, cellHeight: 'sm', sortBy: sortBy },
  transformations: [{
    id: 'organize',
    options: {
      excludeByName: { Time: true, Value: true, __name__: true, job: true, instance: true } + exclude,
      renameByName: rename,
    },
  }],
};

local link(field, title, url) = {
  matcher: { id: 'byName', options: field },
  properties: [{ id: 'links', value: [{ title: title, url: url, targetBlank: true }] }],
};

{
  // =============== LAST NIGHT ===============

  verdict: stat(
    'LAST NIGHT',
    'The last run\'s verdict. GOOD: both units succeeded, the system and the home profile are both on main, and the capture worked. The CHECKS panel says which part failed; the upgrade-log issue says why.',
    [{ expr: 'upgrade_run_good', refId: 'A' }],
    'none',
    [{ color: unknown, value: null }],
    yesNo('NOT GOOD', 'GOOD'),
  ),

  sinceRun: stat(
    'SINCE LAST RUN',
    'Hours since the last nightly run was judged. One runs every night at 04:40 (±20 min); past 36h a night was missed: box off at 04:40 and not caught up, timer broken, run hung, or the reporter failed.',
    [{ expr: '(time() - upgrade_last_run_timestamp_seconds) / 3600', refId: 'A' }],
    'h',
    [{ color: 'green', value: null }, { color: 'red', value: 36 }],
  ) { fieldConfig+: { defaults+: { decimals: 0 } } },

  sinceGood: stat(
    'SINCE GOOD RUN',
    'Hours since the last GOOD run. Past 36h, merges to main are not reaching the box, or are reaching it unverified.',
    [{ expr: '(time() - upgrade_last_good_run_timestamp_seconds) / 3600', refId: 'A' }],
    'h',
    [{ color: 'green', value: null }, { color: 'red', value: 36 }],
  ) { fieldConfig+: { defaults+: { decimals: 0 } } },

  findings: stat(
    'FINDINGS',
    'Warning and error lines the last run printed, after the filter. Healthy is 0; the lines are in the table below and in the upgrade-log issue.',
    [{ expr: 'upgrade_findings', refId: 'A' }],
    'none',
    [{ color: 'green', value: null }, { color: 'orange', value: 1 }],
  ),

  nearMisses: stat(
    'NEAR MISSES',
    'Lines that look like trouble (warn, fail, cannot, ...) that the filter did not keep. Non-zero: read them in the issue, and if one should have been kept, add it to dotfiles\' nixos/auto-upgrade/filter-fixture.txt and fix filter.sh.',
    [{ expr: 'upgrade_near_misses', refId: 'A' }],
    'none',
    [{ color: 'green', value: null }, { color: 'orange', value: 1 }],
  ),

  github: stat(
    'GITHUB',
    'Whether the last run\'s Deployment record and issue reached GitHub. NO: the off-box SLO watcher will report the night as missing. Check the token (Issues and Deployments: read/write).',
    [{ expr: 'upgrade_github_ok', refId: 'A' }],
    'none',
    [{ color: unknown, value: null }],
    yesNo(),
  ),

  metricsFile: stat(
    'METRICS FILE',
    'Whether node_exporter could read the textfile directory every upgrade_* series comes from. ERROR or no data: nothing on this dashboard is current.',
    [{ expr: 'max(node_textfile_scrape_error)', refId: 'A' }],
    'none',
    [{ color: unknown, value: null }],
    [{ type: 'value', options: { '0': { text: 'READ', color: 'green' }, '1': { text: 'ERROR', color: 'red' } } }],
  ),

  checks: stat(
    'CHECKS',
    'Each part of the verdict for the last run. nixos-upgrade, home-manager-upgrade: the unit succeeded. system_on_main: the running system was built from main. home_on_system_rev: the home profile is that commit\'s exact store path. capture: the canary came back through the journal and the filter.',
    [{ expr: 'max by (check) (upgrade_check)', legendFormat: '{{check}}', refId: 'A' }],
    'none',
    [{ color: unknown, value: null }],
    yesNo(),
    'name_and_value',
  ) {
    // Five values side by side at a fixed size; auto-sizing shrinks them to
    // unreadable in a short panel.
    options+: { orientation: 'vertical', text: { titleSize: 13, valueSize: 28 } },
  },

  deployed: table(
    'DEPLOYED',
    'The commit the system was built from, and main\'s tip, when the last run was judged. Different and not GOOD: main has not landed. Click a commit to open it.',
    'upgrade_deployed_info',
    {},
    { rev: 'system runs', main: 'main is' },
    [
      link('system runs', 'commit', repo + '/commit/${__value.raw}'),
      link('main is', 'commit', repo + '/commit/${__value.raw}'),
    ],
  ),

  // =============== FINDINGS ===============

  findingLines: table(
    'LAST RUN\'S FINDINGS',
    'The curated warning and error lines of the last run, in order (first 30, cut at 200 characters). Empty with FINDINGS at 0 is a quiet run; empty with FINDINGS above 0 means the series are missing.',
    'upgrade_finding_info',
    {},
    { n: '#', text: 'line' },
    [{ matcher: { id: 'byName', options: '#' }, properties: [{ id: 'custom.width', value: 50 }] }],
    [{ displayName: '#', desc: false }],
  ),

  report: table(
    'REPORT',
    'The upgrade-log issue holding the last run\'s full report: the checks, every finding, the log tail on failure, near misses, and how to reproduce. Empty: nothing to report, or GitHub was not reached.',
    'upgrade_report_info',
    {},
    { issue: 'issue' },
    [link('issue', 'open the issue', '${__value.raw}')],
  ),

  // =============== OVER TIME ===============

  goodNights: {
    title: 'GOOD NIGHTS',
    description: 'Share of nights with a GOOD run, over the last 7, 30 and 90 days (or since the first recorded run). A night with no run counts against it. The SLO is 95% over 90 nights, about four bad or missing nights.',
    type: 'bargauge',
    datasource: ds,
    // One target per window, so the bars read 7d, 30d, 90d top to bottom.
    targets: [
      {
        expr: 'upgrade_window_nights_good{window="%s"} / upgrade_window_nights{window="%s"}' % [w, w],
        legendFormat: w,
        instant: true,
        refId: std.char(65 + i),
      }
      for i in std.range(0, 2)
      for w in [['7d', '30d', '90d'][i]]
    ],
    fieldConfig: {
      defaults: {
        unit: 'percentunit',
        min: 0,
        max: 1,
        decimals: 1,
        color: { mode: 'thresholds' },
        thresholds: {
          mode: 'absolute',
          steps: [{ color: 'red', value: null }, { color: 'orange', value: 0.9 }, { color: 'green', value: 0.95 }],
        },
      },
      overrides: [],
    },
    options: {
      orientation: 'horizontal',
      displayMode: 'gradient',
      showUnfilled: true,
      reduceOptions: { calcs: ['lastNotNull'], values: false },
    },
  },

  nightCounts: {
    title: 'NIGHTS',
    description: 'The counts behind GOOD NIGHTS: nights in the window, nights with any run, nights with a GOOD run. At one run a night the rate moves in steps, so read it with the count.',
    type: 'table',
    datasource: ds,
    // `max by (window)` drops __name__ and the scrape labels, so the merge
    // below can join the three on `window` alone.
    targets: [
      { expr: 'max by (window) (upgrade_window_nights)', format: 'table', instant: true, refId: 'A' },
      { expr: 'max by (window) (upgrade_window_nights_ran)', format: 'table', instant: true, refId: 'B' },
      { expr: 'max by (window) (upgrade_window_nights_good)', format: 'table', instant: true, refId: 'C' },
    ],
    fieldConfig: { defaults: { custom: { align: 'left' } }, overrides: [] },
    options: { showHeader: true, cellHeight: 'sm' },
    transformations: [
      { id: 'merge', options: {} },
      {
        id: 'organize',
        options: {
          excludeByName: { Time: true },
          indexByName: { window: 0, 'Value #A': 1, 'Value #B': 2, 'Value #C': 3 },
          renameByName: { 'Value #A': 'nights', 'Value #B': 'ran', 'Value #C': 'good' },
        },
      },
      // Lexical order would put 30d first; sort by the window's length.
      { id: 'sortBy', options: { sort: [{ field: 'nights', desc: false }] } },
    ],
  },

  timeline: {
    title: 'VERDICT AND CHECKS, NIGHT BY NIGHT',
    description: 'Each lane holds the last run\'s value until the next run replaces it, so a band is a night. Gaps are not measured (box or scrape down). Prometheus keeps about 8 days here; the long record is the dotfiles repo\'s Deployments page (environment nixos).',
    type: 'state-timeline',
    datasource: ds,
    targets: [
      { expr: 'max(upgrade_run_good)', legendFormat: 'GOOD', refId: 'A' },
      { expr: 'max by (check) (upgrade_check)', legendFormat: '{{check}}', refId: 'B' },
    ],
    fieldConfig: {
      defaults: {
        color: { mode: 'thresholds' },
        thresholds: { mode: 'absolute', steps: [{ color: 'red', value: null }, { color: 'green', value: 1 }] },
        mappings: yesNo(),
        custom: { fillOpacity: 75, lineWidth: 0, spanNulls: false },
      },
      overrides: [],
    },
    options: {
      showValue: 'never',
      mergeValues: true,
      rowHeight: 0.8,
      alignValue: 'left',
      legend: { showLegend: false },
      tooltip: { mode: 'single', sort: 'none' },
    },
  },
}
