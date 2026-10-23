// mailbox-panels.libsonnet
//
// The MAILBOX dashboard's panels. The Jobs mailbox (paulgsc/some-ui
// `apps/mailbox`) is where the owner's cloud routine leaves each day's job
// digest; file_host's `mailbox` poller reads it from there so the phone can
// take today's picks from home. Every series here comes from
// apps/servers/file_host/src/metrics/mailbox.rs.
//
// The second row is the mailbox's D1 database against Cloudflare's free-plan
// daily limits, from apps/servers/file_host/src/metrics/d1_usage.rs. Past a
// limit every mailbox query fails until 00:00 UTC, so the row shows how close
// today is. A number older than three collector intervals is shown as no
// data (grey), never as the last value it had: a stale zero is not a quiet day.
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

local gauge(title, description, target, unit, max) = {
  title: title,
  description: description,
  type: 'gauge',
  datasource: ds,
  targets: [target { instant: true, refId: 'A' }],
  fieldConfig: {
    defaults: {
      unit: unit,
      min: 0,
      max: max,
      color: { mode: 'thresholds' },
      thresholds: {
        mode: 'absolute',
        steps: [{ color: 'green', value: null }, { color: 'orange', value: 0.5 * max }, { color: 'red', value: 0.8 * max }],
      },
      noValue: 'no data',
    },
    overrides: [],
  },
  options: { reduceOptions: { calcs: ['lastNotNull'], values: false }, showThresholdMarkers: true },
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
    'Files kept, refused and handed over',
    'Per hour. Kept and refused: digests read from the mailbox. Handed: the phone\'s voice-free decisions and scouts given to the mailbox, by its answer; a conflict or refusal is final and goes by the share sheet instead.',
    [
      { expr: 'sum(increase(mailbox_poll_items_kept_total[1h]))', legendFormat: 'kept', refId: 'A' },
      { expr: 'sum by (why) (increase(mailbox_poll_items_refused_total[1h]))', legendFormat: 'refused: {{why}}', refId: 'B' },
      { expr: 'sum by (answer) (increase(mailbox_poll_items_handed_total[1h]))', legendFormat: 'handed: {{answer}}', refId: 'C' },
    ],
    'none',
  ),

  passDuration: series(
    'How long a pass takes',
    "The last pass's duration, its requests to the mailbox included. Each request gives up after 30 seconds.",
    [{ expr: 'mailbox_poll_last_pass_duration_seconds', legendFormat: 'last pass', refId: 'A' }],
    's',
  ),

  // ── The database's free allowance ─────────────────────────────────────
  // `and on() (… < 3 * interval)` drops a number whose last good read is
  // older than three intervals, so it renders grey (panel-defaults).

  usageReading: stat(
    'USAGE READ',
    'Whether this home asks Cloudflare for the mailbox database\'s usage (CF_USAGE_ENABLED). OFF is a choice, not a fault.',
    { expr: 'd1_usage_enabled' },
    'none',
    [{ color: 'green', value: null }],
    [{ type: 'value', options: { '0': { text: 'OFF', color: '#8E8E8E' }, '1': { text: 'ON', color: 'green' } } }],
  ),

  rowsWritten: gauge(
    'Rows written today, of 100,000 allowed',
    'The UTC day so far. At 100,000 every mailbox query fails until 00:00 UTC. Grey: the numbers are stale.',
    { expr: 'd1_usage_rows_written_today and on() ((time() - d1_usage_last_success_timestamp_seconds) < 3 * d1_usage_interval_seconds)' },
    'none',
    100000,
  ),

  rowsRead: gauge(
    'Rows read today, of 5,000,000 allowed',
    'The UTC day so far. At 5,000,000 every mailbox query fails until 00:00 UTC. Grey: the numbers are stale.',
    { expr: 'd1_usage_rows_read_today and on() ((time() - d1_usage_last_success_timestamp_seconds) < 3 * d1_usage_interval_seconds)' },
    'none',
    5000000,
  ),

  databaseSize: gauge(
    'Database size, of 500 MB allowed',
    'What the mailbox holds. It only grows: the mailbox is append-only.',
    { expr: 'd1_usage_database_bytes and on() ((time() - d1_usage_last_success_timestamp_seconds) < 3 * d1_usage_interval_seconds)' },
    'decbytes',
    500000000,
  ),

  usageFreshness: stat(
    'NUMBERS AGE',
    'Collector intervals since Cloudflare last answered. Above 3 the numbers above are grey; the panel below says why.',
    { expr: '(time() - d1_usage_last_success_timestamp_seconds) / d1_usage_interval_seconds' },
    'none',
    [{ color: 'green', value: null }, { color: 'red', value: 3 }],
  ) { fieldConfig+: { defaults+: { decimals: 1 } } },

  usageFailingByWhy: series(
    'Why the last usage read failed',
    'One line per reason; 1 while the last read failed for it. unauthorized: CF_API_TOKEN cannot read the account. api: Cloudflare refused the query. The dotted line is intervals since the collector last asked at all.',
    [
      { expr: 'max by (error) (d1_usage_failing)', legendFormat: '{{error}}', refId: 'A' },
      { expr: '(time() - d1_usage_last_attempt_timestamp_seconds) / d1_usage_interval_seconds', legendFormat: 'intervals since asked', refId: 'B' },
    ],
    'none',
  ),

  rowsToday: series(
    'Rows today, through the day',
    'Each line resets at 00:00 UTC, when the free plan\'s day does. A line climbing every few minutes is a runaway writer.',
    [
      { expr: 'd1_usage_rows_written_today', legendFormat: 'written', refId: 'A' },
      { expr: 'd1_usage_rows_read_today', legendFormat: 'read', refId: 'B' },
    ],
    'none',
  ),

  queriesToday: series(
    'Queries today, by kind',
    'Read and write queries the Worker made today.',
    [{ expr: 'd1_usage_queries_today', legendFormat: '{{kind}}', refId: 'A' }],
    'none',
  ),

  queryLatency: series(
    'Query time, 90th percentile today',
    'How long the Worker\'s queries took, today; 0 on a day with none.',
    [{ expr: 'd1_usage_query_batch_p90_milliseconds', legendFormat: 'p90', refId: 'A' }],
    'ms',
  ),
}
