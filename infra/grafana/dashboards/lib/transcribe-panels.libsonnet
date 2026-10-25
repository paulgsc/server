// transcribe-panels.libsonnet
//
// The TRANSCRIBE dashboard's panels. file_host's `transcribe` worker hears
// each voice note the phone sends home with whisper.cpp on home's CPU and
// keeps what it heard in `voice_transcript` (paulgsc/server#462). Every
// series here comes from apps/servers/file_host/src/metrics/transcribe.rs,
// and none names a note or a subject: labels are attempt kinds only.
//
// As on MAILBOX, the healthy state is quiet: notes arrive a few a day, and
// most passes find nothing to hear. So HEARING says whether the deployment
// runs the worker at all, SINCE LAST LOOK whether it is still asking, and
// FAILING whether the last pass ended on a storage failure. No data stays
// grey (panel-defaults).

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
  hearing: stat(
    'HEARING',
    'Whether this home transcribes voice notes at all (TRANSCRIBE_ENABLED). OFF is a choice, not a fault: notes are kept unheard.',
    { expr: 'transcribe_enabled' },
    'none',
    [{ color: 'green', value: null }],
    [{ type: 'value', options: { '0': { text: 'OFF', color: '#8E8E8E' }, '1': { text: 'ON', color: 'green' } } }],
  ),

  // In seconds rather than intervals: a pass that hears notes runs as long
  // as they take, so a few minutes is a busy pass, and an hour is a stuck one.
  sinceLastLook: stat(
    'SINCE LAST LOOK',
    'Time since the worker last started a pass. Every TRANSCRIBE_SECONDS (60) when idle; longer while it hears a batch. Past an hour, the task died or a note hangs past its deadline.',
    { expr: 'time() - transcribe_last_attempt_timestamp_seconds' },
    's',
    [{ color: 'green', value: null }, { color: 'red', value: 3600 }],
  ),

  failing: stat(
    'FAILING',
    'Whether the last pass ended on a storage failure: 0 is healthy. A note that cannot be heard is not a failure here; it is kept as an attempt.',
    { expr: 'transcribe_pass_failing' },
    'none',
    [{ color: 'green', value: null }, { color: 'red', value: 1 }],
  ),

  heardToday: stat(
    'NOTES HEARD, 24H',
    'Notes heard (or found silent) in the last 24 hours. Zero is normal on a day nothing was recorded.',
    { expr: 'sum(increase(transcribe_attempts_total{outcome=~"heard|silent"}[24h]))' },
    'none',
    [{ color: 'green', value: null }],
  ) { fieldConfig+: { defaults+: { decimals: 0 } } },

  notHeardToday: stat(
    'NOT HEARD, 24H',
    'Attempts in the last 24 hours that did not hear a note: unreadable, too long, past the deadline, or an engine failure. Healthy is 0; a note is tried at most three times per model.',
    { expr: 'sum(increase(transcribe_attempts_total{outcome!~"heard|silent"}[24h]))' },
    'none',
    [{ color: 'green', value: null }, { color: 'red', value: 1 }],
  ) { fieldConfig+: { defaults+: { decimals: 0 } } },

  attemptsByOutcome: series(
    'Attempts, by outcome',
    'Per hour. heard and silent end a note; unreadable, too_long, deadline and failed are kept and retried a pass later, at most three times.',
    [{ expr: 'sum by (outcome) (increase(transcribe_attempts_total[1h]))', legendFormat: '{{outcome}}', refId: 'A' }],
    'none',
  ),

  attemptDuration: series(
    'How long hearing a note takes',
    'Mean seconds per attempt over the hour, by outcome: decoding, finding the speech and Whisper together. About the note\'s own length on two threads; TRANSCRIBE_DEADLINE_SECONDS bounds it.',
    [{
      expr: 'sum by (outcome) (rate(transcribe_attempt_seconds_sum[1h])) / sum by (outcome) (rate(transcribe_attempt_seconds_count[1h]))',
      legendFormat: '{{outcome}}',
      refId: 'A',
    }],
    's',
  ),
}
