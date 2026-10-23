// mailbox-panels.libsonnet
//
// The MAILBOX dashboard's panels. The Jobs mailbox (paulgsc/some-ui
// `apps/mailbox`) is where the owner's cloud routine leaves each day's job
// digest; file_host's `mailbox` poller reads it from there so the phone can
// take today's picks from home. Every series here comes from
// apps/servers/file_host/src/metrics/mailbox.rs.
//
// The healthy state is quiet: one digest a day, most passes finding nothing.
// So, as on the NUDGE row, "nothing kept" must not read like "nothing
// running": READING says whether the deployment runs the poller at all,
// SINCE LAST READ says whether it is still asking, and FAILING says whether
// the last pass ended early and why. No data stays grey (panel-defaults).

local ds = { type: 'prometheus', uid: 'prometheus' };

local stat(title, description, target, unit, steps, mappings=[]) = {
  title: title,
  description: description,
  type: 'stat',
  datasource: ds,
  targets: [target { instant: true, refId: 'A' }],
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
    textMode: 'value',
  },
};

local series(title, description, targets, unit) = {
  title: title,
  description: description,
  type: 'timeseries',
  datasource: ds,
  targets: targets,
  fieldConfig: {
    defaults: {
      unit: unit,
      custom: { drawStyle: 'line', lineWidth: 1, fillOpacity: 10, spanNulls: false },
    },
    overrides: [],
  },
  options: { legend: { displayMode: 'list', placement: 'bottom' }, tooltip: { mode: 'multi' } },
};

{
  reading: stat(
    'READING',
    'Whether this home reads the Jobs mailbox at all (MAILBOX_ENABLED). OFF is a choice, not a fault.',
    { expr: 'mailbox_poll_enabled' },
    'none',
    [{ color: 'green', value: null }],
    [{ type: 'value', options: { '0': { text: 'OFF', color: '#8E8E8E' }, '1': { text: 'ON', color: 'green' } } }],
  ),

  // In intervals rather than seconds, so the threshold holds whatever
  // MAILBOX_SECONDS is: a pass is due every interval, and two missed is stuck.
  sinceLastRead: stat(
    'SINCE LAST READ',
    'Intervals since the poller last started a pass. Above 2, it has stopped asking: the task died or a pass hangs.',
    { expr: '(time() - mailbox_poll_last_attempt_timestamp_seconds) / mailbox_poll_interval_seconds' },
    'none',
    [{ color: 'green', value: null }, { color: 'red', value: 2 }],
  ) { fieldConfig+: { defaults+: { decimals: 1 } } },

  failing: stat(
    'FAILING',
    'How many reasons the last pass ended early: 0 is healthy. The panel below names the reason.',
    { expr: 'sum(mailbox_poll_failing)' },
    'none',
    [{ color: 'green', value: null }, { color: 'red', value: 1 }],
  ),

  keptToday: stat(
    'DIGESTS KEPT, 24H',
    'Files home kept from the mailbox in the last 24 hours. One a day is the routine working; zero for a whole day means no digest reached home.',
    { expr: 'sum(increase(mailbox_poll_items_kept_total[24h]))' },
    'none',
    [{ color: 'red', value: null }, { color: 'green', value: 1 }],
  ) { fieldConfig+: { defaults+: { decimals: 0 } } },

  refusedToday: stat(
    'FILES REFUSED, 24H',
    'Files home read and would not keep in the last 24 hours: bytes not as listed, not JSON, or a held name with other bytes. Healthy is 0.',
    { expr: 'sum(increase(mailbox_poll_items_refused_total[24h]))' },
    'none',
    [{ color: 'green', value: null }, { color: 'red', value: 1 }],
  ) { fieldConfig+: { defaults+: { decimals: 0 } } },

  failingByWhy: series(
    'Why the last pass ended early',
    'One line per reason; 1 while the last pass failed for it. no_account means MAILBOX_SUBJECT names no account; unauthorized, a wrong MAILBOX_TOKEN.',
    [{ expr: 'max by (error) (mailbox_poll_failing)', legendFormat: '{{error}}', refId: 'A' }],
    'none',
  ),

  keptAndRefused: series(
    'Files kept and refused',
    'Per hour.',
    [
      { expr: 'sum(increase(mailbox_poll_items_kept_total[1h]))', legendFormat: 'kept', refId: 'A' },
      { expr: 'sum by (why) (increase(mailbox_poll_items_refused_total[1h]))', legendFormat: 'refused: {{why}}', refId: 'B' },
    ],
    'none',
  ),

  passDuration: series(
    'How long a pass takes',
    "The last pass's duration, its requests to the mailbox included. Each request gives up after 30 seconds.",
    [{ expr: 'mailbox_poll_last_pass_duration_seconds', legendFormat: 'last pass', refId: 'A' }],
    's',
  ),
}
