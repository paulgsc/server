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

  archiving: stat(
    'ARCHIVING',
    'Whether this home archives old mailbox files (MAILBOX_ARCHIVE_TOKEN). OFF is a choice: the mailbox then keeps every file live.',
    { expr: 'mailbox_archive_enabled' },
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
    'One line per reason; 1 while the last pass failed for it. no_account means there is no account to keep files for (MAILBOX_SUBJECT names none, or it is unset and home has no account yet, or several); unauthorized, a wrong MAILBOX_TOKEN.',
    [{ expr: 'max by (error) (mailbox_poll_failing)', legendFormat: '{{error}}', refId: 'A' }],
    'none',
  ),

  keptAndRefused: series(
    'Files kept, refused, handed over, archived and put back',
    'Per hour. Kept and refused: digests read from the mailbox. Handed: the phone\'s voice-free decisions and scouts given to the mailbox, by its answer; a conflict or refusal is final and goes by the share sheet instead. Archived: files live in the mailbox for MAILBOX_ARCHIVE_DAYS, stored home and then dropped there, or why one stayed live. Put back: archived files the routine asked for again; missing means home holds nothing for the ask. Both stay at 0 without MAILBOX_ARCHIVE_TOKEN.',
    [
      { expr: 'sum(increase(mailbox_poll_items_kept_total[1h]))', legendFormat: 'kept', refId: 'A' },
      { expr: 'sum by (why) (increase(mailbox_poll_items_refused_total[1h]))', legendFormat: 'refused: {{why}}', refId: 'B' },
      { expr: 'sum by (answer) (increase(mailbox_poll_items_handed_total[1h]))', legendFormat: 'handed: {{answer}}', refId: 'C' },
      { expr: 'sum by (answer) (increase(mailbox_poll_items_archived_total[1h]))', legendFormat: 'archived: {{answer}}', refId: 'D' },
      { expr: 'sum by (answer) (increase(mailbox_poll_items_restored_total[1h]))', legendFormat: 'put back: {{answer}}', refId: 'E' },
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
