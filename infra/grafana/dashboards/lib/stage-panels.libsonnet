// stage-panels.libsonnet
//
// SHARED STAGES (paulgsc/some-ui#3333): the top row of the file_host
// dashboard. A shared stage is a node other pipelines wait on
// (`crate::stage::Stage` in file_host), and each one here gets the same
// three tiles, from the same series, told apart by the `stage` label:
//
//   STATE         OK / SLOW / STUCK: slow while the oldest item waiting is
//                 older than the stage's promise (StageDegraded's test),
//                 stuck while it makes more attempts in an hour than one item
//                 gets and almost nothing ends (StageBroken's).
//                 Read now, not held as the alerts are, so it can show SLOW a
//                 few minutes before the alert reaches the owner.
//   OLDEST        how long the oldest item has waited, with its trend.
//   WHO WAITS     what is blocked downstream, one value per waiter the stage
//                 declares (`Stage::waiters`).
//
// A stage that has never said what waits (switched off, or file_host not
// scraped) reads NO DATA, never OK (docs/dashboard-honesty.md).
//
// `stages` is the one list a stage is added to here; file_host's
// `stage::tests` fails while a case of `Stage` has no entry, or an entry
// does not read each of its waiters' series.
local glance = import 'glance.libsonnet';
local utils = import 'utils.libsonnet';

local ds = { type: 'prometheus', uid: 'prometheus' };

local stages = [
  {
    stage: 'transcriber',
    title: 'TRANSCRIBER',
    promise: 'a note is heard within 15 minutes of reaching home (VH7)',
    // Stage::Transcriber's waiters: Waiter::CardsEssays, Waiter::RenderLane.
    waiters: [
      {
        expr: 'max(stage_waiting{stage="transcriber"})',
        legend: 'notes waiting',
        unit: 'none',
      },
      {
        expr: 'max(cards_essays_waiting_on_takes)',
        legend: 'cards essays held for a take',
        unit: 'none',
      },
      {
        // The share of the last hour's lane ticks held for interactive
        // work, by kind: hearing notes, recording programs. Held means no
        // render may start, whether or not one was waiting. Capped at 1: a
        // (verb, why) pair is counted from its first tick, so just after a
        // restart the share can read a little high.
        expr: 'clamp_max(sum by (kind) (increase(lane_busy_ticks_total[1h])) / scalar(sum(increase(lane_ticks_total[1h]))), 1)',
        legend: 'lane held for it, last hour: {{kind}}',
        unit: 'percentunit',
      },
    ],
  },
];

local selector(s) = 'stage="%s"' % s.stage;
local attempts(s, also='') = 'sum(increase(stage_attempts_total{%s%s}[1h]))' % [selector(s), also];
local slow(s) = '(max(stage:oldest_waiting:seconds{%s}) > bool max(stage_promise_seconds{%s}))' % [selector(s), selector(s)];
local broken(s) = '((%s > bool (max(stage_attempts_per_item{%s}) + 0.5)) * (%s < bool 0.1 * %s))' % [attempts(s), selector(s), attempts(s, ',result=~"done|refused"'), attempts(s)];
// 2 while broken, else 1 while slow, else 0. Built here, not in an `expr:`
// string, so scripts/check_metric_contract.py does not read `%s` as a
// metric; the names it reads are the alerts', in infra/prometheus/rules.
local verdict(s) = '2 * %s + %s * (1 - %s)' % [broken(s), slow(s), broken(s)];

local state(s) = {
  title: s.title,
  description: 'Is the %s keeping its promise: %s. OK: nothing has waited longer. SLOW: something has (StageDegraded reaches the owner once it holds 5 minutes). STUCK: it made more attempts in the last hour than one item gets, and under a tenth ended (StageBroken). NO DATA: the stage is off, or file_host is not being scraped.' % [std.asciiLower(s.title), s.promise],
  type: 'stat',
  datasource: ds,
  targets: [{ expr: verdict(s), instant: true, refId: 'A' }],
  fieldConfig: {
    defaults: {
      color: { mode: 'thresholds' },
      thresholds: glance.steps([{ color: glance.green, value: null }, { color: glance.amber, value: 1 }, { color: glance.red, value: 2 }]),
      mappings: [{
        type: 'value',
        options: {
          '0': { text: 'OK', index: 0 },
          '1': { text: 'SLOW', index: 1 },
          '2': { text: 'STUCK', index: 2 },
        },
      }],
      noValue: 'NO DATA',
    },
    overrides: [],
  },
  options: {
    colorMode: 'background',
    graphMode: 'none',
    justifyMode: 'center',
    orientation: 'auto',
    textMode: 'value',
    reduceOptions: { calcs: ['lastNotNull'], values: false },
  },
};

local oldest(s) = glance.sparkStat(
  'OLDEST WAITING',
  'How long the oldest item still waiting on the %s has waited (0: nothing waits). It catches stopped, slow, looping, stuck on one item and backlogged alike; STATE turns SLOW when it passes the promise.' % std.asciiLower(s.title),
  'max(stage:oldest_waiting:seconds{%s})' % selector(s),
  's',
  glance.informational,
) { fieldConfig+: { defaults+: { noValue: 'NO DATA' } } };

local whoWaits(s) = {
  title: 'WHO WAITS',
  description: 'What waits downstream of the %s, one value per waiter it declares (Stage::waiters in file_host): how many items it holds, what downstream work is held back for them, and how much of the last hour the batch lane paused renders for interactive work, by kind.' % std.asciiLower(s.title),
  type: 'stat',
  datasource: ds,
  targets: [
    { expr: w.expr, legendFormat: w.legend, instant: true, refId: std.char(65 + i) }
    for i in std.range(0, std.length(s.waiters) - 1)
    for w in [s.waiters[i]]
  ],
  fieldConfig: {
    defaults: {
      color: { mode: 'thresholds' },
      thresholds: glance.informational,
      noValue: 'NO DATA',
      decimals: 0,
    },
    overrides: [
      {
        matcher: { id: 'byFrameRefID', options: std.char(65 + i) },
        properties: [{ id: 'unit', value: s.waiters[i].unit }],
      }
      for i in std.range(0, std.length(s.waiters) - 1)
    ],
  },
  options: {
    colorMode: 'value',
    graphMode: 'none',
    justifyMode: 'center',
    orientation: 'vertical',
    textMode: 'value_and_name',
    wideLayout: true,
    reduceOptions: { calcs: ['lastNotNull'], values: false },
  },
};

{
  stages:: stages,

  // How many grid rows the SHARED STAGES row takes, its header included.
  height:: 1 + 4 * std.length(stages),

  // The row's header and each stage's three tiles, from `y`, ids from `id`.
  row(y, id):: [
    { type: 'row', title: 'Shared stages: what other work waits on', id: id, collapsed: false, gridPos: utils.gridPos(0, y, 24, 1), panels: [] },
  ] + std.flattenArrays([
    local s = stages[i];
    local top = y + 1 + 4 * i;
    local base = id + 1 + 3 * i;
    [
      state(s) { gridPos: utils.gridPos(0, top, 4, 4), id: base },
      oldest(s) { gridPos: utils.gridPos(4, top, 6, 4), id: base + 1 },
      whoWaits(s) { gridPos: utils.gridPos(10, top, 14, 4), id: base + 2 },
    ]
    for i in std.range(0, std.length(stages) - 1)
  ]),
}
