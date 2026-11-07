// call-panels.libsonnet
//
// The CALL dashboard's panels: the weekly call home keeps and makes into a
// video (paulgsc/some-ui#2635, ST36). Every series here comes from
// apps/servers/file_host/src/metrics/call_weeks.rs: file_host's call weeks
// worker (CW2) and what it reads, each minute, of the render worker's
// folders on home's database drive (call_weeks::render, CW5). No label
// carries a week; which week failed is the file named in FAILED's text.
//
// Healthy is quiet: weeks held, none waiting, both workers stamping within
// their hour. The Call* alerts in infra/prometheus/rules/home.yml say the
// same to Slack.

local ds = { type: 'prometheus', uid: 'prometheus' };

local stat(title, description, expr, unit, steps, mappings=[]) = {
  title: title,
  description: description,
  type: 'stat',
  datasource: ds,
  targets: [{ expr: expr, instant: true, refId: 'A' }],
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

local count(title, description, expr, steps) =
  stat(title, description, expr, 'none', steps) { fieldConfig+: { defaults+: { decimals: 0 } } };

{
  held: count(
    'WEEKS HELD',
    'Weeks of the call home keeps (call_week), each for good.',
    'call_weeks_held',
    [{ color: 'green', value: null }],
  ),

  sinceRead: stat(
    'SINCE LAST READ',
    "Time since home last read paulgsc/testing's index of weeks. It reads hourly, sooner after a failure; past 2 hours CallWeeksStale fires.",
    'time() - call_weeks_last_attempt_timestamp_seconds',
    's',
    [{ color: 'green', value: null }, { color: 'red', value: 7200 }],
  ),

  readFailing: stat(
    'READ FAILING',
    "Whether home's last read of the weeks failed, on storage or on a week it could not read or write: 0 is healthy. file-host-server's log names what failed.",
    'call_weeks_pass_failing',
    'none',
    [{ color: 'green', value: null }, { color: 'red', value: 1 }],
  ),

  waiting: count(
    'WAITING FOR A VIDEO',
    'Weeks home holds with no video yet in renders/call/<week>/ on its database drive. A new week waits until the render worker next passes, within the hour.',
    'call_render_weeks_waiting',
    [{ color: 'green', value: null }, { color: 'orange', value: 1 }],
  ),

  failed: count(
    'FAILED',
    "Of those, weeks whose last attempt failed. Its whole output is renders/call-worker/<week>.failed.log on home's database drive, until the week renders.",
    'call_render_weeks_failed',
    [{ color: 'green', value: null }, { color: 'red', value: 1 }],
  ),

  longestWait: stat(
    'LONGEST WAIT',
    'How long the week that has waited longest has been held without a video; 0 when none waits. Past 3 hours with no failed attempt, CallRenderOverdue fires.',
    '(time() - call_render_oldest_waiting_taken_timestamp_seconds) * (call_render_weeks_waiting > bool 0)',
    's',
    [{ color: 'green', value: null }, { color: 'orange', value: 3600 }, { color: 'red', value: 10800 }],
  ),

  sinceWorker: stat(
    'SINCE RENDER PASS',
    'Time since the render worker last started a pass. It starts one an hour after the last ends; past 4 hours CallRenderWorkerStale fires. No data: the worker has never run.',
    '(time() - call_render_worker_last_pass_timestamp_seconds) and (call_render_worker_last_pass_timestamp_seconds > 0)',
    's',
    [{ color: 'green', value: null }, { color: 'orange', value: 7200 }, { color: 'red', value: 14400 }],
  ),

  restarts: count(
    'CRASHES, 24H',
    'Times the speaker, the stage or the render worker died and Docker started the same container again in the last 24 hours. Each start resets the container\'s CPU counter; a deploy is a new container, not one of these. CallServiceRestarted fires on each.',
    'sum(max by (name) (resets(container_cpu_usage_seconds_total{name=~"kokoro|call-stage|render"}[24h])))',
    [{ color: 'green', value: null }, { color: 'red', value: 1 }],
  ),

  memory: {
    title: 'Memory of the call\'s containers, share of their cap',
    description: 'Working set (what the kernel will not reclaim) of the speaker, the stage and the render worker, as a share of each container\'s mem_limit. At 100% the kernel kills it: the speaker did, four times on Nov 6, twelve lines into a week.',
    type: 'timeseries',
    datasource: ds,
    targets: [
      {
        expr: '100 * max by (name) (container_memory_working_set_bytes{name=~"kokoro|call-stage|render"}) / max by (name) (container_spec_memory_limit_bytes{name=~"kokoro|call-stage|render"} > 0)',
        legendFormat: '{{name}}',
        refId: 'A',
      },
    ],
    fieldConfig: {
      defaults: {
        unit: 'percent',
        min: 0,
        max: 100,
        custom: { drawStyle: 'line', lineWidth: 1, fillOpacity: 10, spanNulls: false, thresholdsStyle: { mode: 'line' } },
        thresholds: { mode: 'absolute', steps: [{ color: 'green', value: null }, { color: 'orange', value: 80 }, { color: 'red', value: 95 }] },
      },
      overrides: [],
    },
    options: { legend: { displayMode: 'list', placement: 'bottom' }, tooltip: { mode: 'multi' } },
  },

  weeks: {
    title: 'Weeks held, waiting and failed',
    description: 'Over time: a week arrives, waits for the render worker, and leaves waiting once its video is kept.',
    type: 'timeseries',
    datasource: ds,
    targets: [
      { expr: 'call_weeks_held', legendFormat: 'held', refId: 'A' },
      { expr: 'call_render_weeks_waiting', legendFormat: 'waiting', refId: 'B' },
      { expr: 'call_render_weeks_failed', legendFormat: 'failed', refId: 'C' },
    ],
    fieldConfig: {
      defaults: {
        unit: 'none',
        decimals: 0,
        custom: { drawStyle: 'line', lineInterpolation: 'stepAfter', lineWidth: 1, fillOpacity: 10, spanNulls: false },
      },
      overrides: [],
    },
    options: { legend: { displayMode: 'list', placement: 'bottom' }, tooltip: { mode: 'multi' } },
  },
}
