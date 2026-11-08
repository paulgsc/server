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
//
// The render worker's second job, Playoff Delta (paulgsc/some-ui#2647,
// FEED4), has its own row, from metrics/pd_fills.rs: the pd_fills worker
// (PK2) and what the worker has made of each voiced fill (pd_fills::render,
// PK5). The containers and the drive are shared, so their panels are too.

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
    'Weeks home holds with no video yet in renders/call/<week>/ on its database drive, not counting a week you let go from My data. A new week waits until the render worker next passes, within the hour.',
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
    'Times the speaker, the stage or the render worker died and Docker started the same container again in the last 24 hours. Each start resets the container\'s CPU counter; a deploy is a new container, not one of these, and the crashes of every container a service had in the day are added up. CallServiceRestarted fires on each.',
    'sum(max by (name, id) (resets(container_cpu_usage_seconds_total{name=~"kokoro|call-stage|render"}[24h])))',
    [{ color: 'green', value: null }, { color: 'red', value: 1 }],
  ),

  videos: stat(
    'VIDEOS KEPT',
    'Bytes of every video of the call kept for good in renders/call, with what each was made from. Grows by a week a week (CW6).',
    'call_render_videos_bytes',
    'bytes',
    [{ color: 'green', value: null }],
  ),

  work: stat(
    'WORK AND CACHE',
    "Bytes of the waiting weeks' work in renders/call-work: each week's step cache, kept between attempts so a retry runs only what changed, removed once the week renders. The render policy caps each week (--cache-mb) and all of them together.",
    'call_render_work_bytes',
    'bytes',
    [{ color: 'green', value: null }, { color: 'orange', value: 2147483648 }, { color: 'red', value: 4294967296 }],
  ),

  sinceCacheWarning: stat(
    'SINCE CACHE WARNING',
    "Time since the render worker last wrote a cache warning: a step past the policy ran uncached, or a waiting week's work was removed past the total cap. No data: never. Within a day, CallRenderCacheFull says so.",
    '(time() - call_render_cache_warning_timestamp_seconds) and (call_render_cache_warning_timestamp_seconds > 0)',
    's',
    [{ color: 'orange', value: null }, { color: 'green', value: 86400 }],
  ),

  driveFree: stat(
    'DRIVE FREE',
    "Share of home's database drive free: it keeps the database, every video and the caches. Under 10% for ten minutes, HomeDriveLow fires.",
    '100 * call_render_drive_avail_bytes / (call_render_drive_size_bytes > 0)',
    'percent',
    [{ color: 'red', value: null }, { color: 'orange', value: 10 }, { color: 'green', value: 20 }],
  ),

  driveUse: {
    title: 'What the call keeps on the drive, over time',
    description: "The videos kept for good grow by a week a week; the waiting weeks' work and caches come and go as weeks wait and render.",
    type: 'timeseries',
    datasource: ds,
    targets: [
      { expr: 'call_render_videos_bytes', legendFormat: 'videos', refId: 'A' },
      { expr: 'call_render_work_bytes', legendFormat: 'work and caches', refId: 'B' },
    ],
    fieldConfig: {
      defaults: {
        unit: 'bytes',
        custom: { drawStyle: 'line', lineInterpolation: 'stepAfter', lineWidth: 1, fillOpacity: 10, spanNulls: false },
      },
      overrides: [],
    },
    options: { legend: { displayMode: 'list', placement: 'bottom' }, tooltip: { mode: 'multi' } },
  },

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
    title: 'Weeks held, waiting, failed and let go',
    description: 'Over time: a week arrives, waits for the render worker, and leaves waiting once its video is kept. A week whose video you let go from My data, once its YouTube link was recorded, counts as let go, never as waiting.',
    type: 'timeseries',
    datasource: ds,
    targets: [
      { expr: 'call_weeks_held', legendFormat: 'held', refId: 'A' },
      { expr: 'call_render_weeks_waiting', legendFormat: 'waiting', refId: 'B' },
      { expr: 'call_render_weeks_failed', legendFormat: 'failed', refId: 'C' },
      { expr: 'call_render_weeks_let_go', legendFormat: 'let go', refId: 'D' },
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

  pdHeld: count(
    'FILLS HELD',
    "Fills of Playoff Delta home keeps (pd_fill), each with its season, for good.",
    'pd_fills_held',
    [{ color: 'green', value: null }],
  ),

  pdSinceRead: stat(
    'SINCE LAST READ',
    "Time since home last read paulgsc/testing's index of fills. It reads hourly, sooner after a failure; past 2 hours PlayoffDeltaFillsStale fires.",
    'time() - pd_fills_last_attempt_timestamp_seconds',
    's',
    [{ color: 'green', value: null }, { color: 'red', value: 7200 }],
  ),

  pdReadFailing: stat(
    'READ FAILING',
    "Whether home's last read of the fills failed, on storage, on a fill or season it could not read, or on one it could not write for the worker: 0 is healthy.",
    'pd_fills_pass_failing',
    'none',
    [{ color: 'green', value: null }, { color: 'red', value: 1 }],
  ),

  pdWaiting: count(
    'WAITING FOR A VIDEO',
    'Voiced fills home holds with no video yet in renders/playoff-delta/<season>-<team>-<NN>/. A new fill waits until the render worker next passes, within the hour.',
    'pd_render_episodes_waiting',
    [{ color: 'green', value: null }, { color: 'orange', value: 1 }],
  ),

  pdFailed: count(
    'FAILED',
    "Of those, episodes whose last attempt failed. Its whole output is renders/playoff-delta-worker/<key>.failed.log on home's database drive, until it renders.",
    'pd_render_episodes_failed',
    [{ color: 'green', value: null }, { color: 'red', value: 1 }],
  ),

  pdUnvoiced: count(
    'FOR MY VOICE',
    'Fills written for your own voice (format version 1): kept as the record of what was said, never rendered by home.',
    'pd_render_episodes_unvoiced',
    [{ color: 'green', value: null }],
  ),

  pdEpisodes: {
    title: 'Fills held, waiting, failed, for my voice and let go',
    description: "Over time: a fill arrives, a voiced one waits for the render worker and leaves waiting once its video is kept; one written for your own voice is kept and never rendered. An episode whose video you let go from My data, once its YouTube link was recorded, counts as let go, never as waiting.",
    type: 'timeseries',
    datasource: ds,
    targets: [
      { expr: 'pd_fills_held', legendFormat: 'held', refId: 'A' },
      { expr: 'pd_render_episodes_waiting', legendFormat: 'waiting', refId: 'B' },
      { expr: 'pd_render_episodes_failed', legendFormat: 'failed', refId: 'C' },
      { expr: 'pd_render_episodes_unvoiced', legendFormat: 'for my voice', refId: 'D' },
      { expr: 'pd_render_episodes_let_go', legendFormat: 'let go', refId: 'E' },
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

  // ---- LANE: when the render may run (L4b, server#708) ----
  // Every series here is file_host's batch lane (metrics/lane.rs,
  // call_weeks::lane), which decides every 5 s from /proc and home's own
  // interactive work. docs/home-compute.md, "The lane", says what each
  // reason means and home's policy.

  laneNow: {
    title: 'What the lane says, and why',
    description: "Over time, what the lane told the render worker (run, pause, stop) and why: clear, busy (a voice note or a Rust Rounds program is being worked on), memory (free memory under the floor, or under the render's peak above it), pressure (memory pressure past its limit), unknown (a signal could not be read) or settling (room came back and is being held for 30 s).",
    type: 'state-timeline',
    datasource: ds,
    targets: [
      { expr: 'max(lane_verb{verb="pause"}) + 2 * max(lane_verb{verb="stop"})', legendFormat: 'verb', refId: 'A' },
      {
        expr: '10 + max(lane_why{why="busy"}) + 2 * max(lane_why{why="memory"}) + 3 * max(lane_why{why="pressure"}) + 4 * max(lane_why{why="unknown"}) + 5 * max(lane_why{why="settling"})',
        legendFormat: 'why',
        refId: 'B',
      },
    ],
    fieldConfig: {
      defaults: {
        color: { mode: 'thresholds' },
        thresholds: { mode: 'absolute', steps: [{ color: 'text', value: null }] },
        mappings: [{
          type: 'value',
          options: {
            '0': { text: 'run', color: 'green' },
            '1': { text: 'pause', color: 'yellow' },
            '2': { text: 'stop', color: 'orange' },
            '10': { text: 'clear', color: 'green' },
            '11': { text: 'busy', color: 'yellow' },
            '12': { text: 'memory', color: 'orange' },
            '13': { text: 'pressure', color: 'orange' },
            '14': { text: 'unknown', color: 'red' },
            '15': { text: 'settling', color: 'blue' },
          },
        }],
        custom: { fillOpacity: 80, lineWidth: 0 },
      },
      overrides: [],
    },
    options: { showValue: 'auto', mergeValues: true, rowHeight: 0.8, alignValue: 'left', legend: { showLegend: false } },
  },

  laneShare: {
    title: 'Time in each decision',
    description: "How long, over the range, the lane said each verb for each reason: how long the render was held back, and by what. Each decision lasts one 5 s tick.",
    type: 'bargauge',
    datasource: ds,
    targets: [{
      expr: 'sort_desc(5 * sum by (verb, why) (increase(lane_ticks_total[$__range])) > 0)',
      legendFormat: '{{verb}}: {{why}}',
      instant: true,
      refId: 'A',
    }],
    fieldConfig: {
      defaults: { unit: 's', min: 0, color: { mode: 'continuous-BlPu' }, decimals: 0 },
      overrides: [],
    },
    options: { displayMode: 'gradient', orientation: 'horizontal', showUnfilled: true, reduceOptions: { calcs: ['lastNotNull'], values: false } },
  },

  laneSince: stat(
    'SINCE LAST DECISION',
    'Time since the lane last wrote its decision for the render worker. It writes every 5 s; the worker treats one older than 30 s as stop, and past a minute LaneStale fires.',
    'time() - lane_last_decision_timestamp_seconds',
    's',
    [{ color: 'green', value: null }, { color: 'orange', value: 30 }, { color: 'red', value: 60 }],
  ),

  laneStops: stat(
    'STOPS OF THIS JOB',
    "Times the render job in progress was stopped for memory or pressure, in a row. At 3 the lane calls it stuck and LaneStuck fires; a new job starts at 0. 0 with no job in progress.",
    'max(lane_stops)',
    'none',
    [{ color: 'green', value: null }, { color: 'orange', value: 1 }, { color: 'red', value: 3 }],
  ) {
    // Spelled out where scripts/check_metric_contract.py reads it.
    targets: [{ expr: 'max(lane_stops)', instant: true, refId: 'A' }],
    fieldConfig+: { defaults+: { decimals: 0 } },
  },

  laneReadable: stat(
    'SIGNALS READ',
    "Whether the lane read every signal it decides from this tick: /proc/meminfo, /proc/pressure/memory (PSI), and the render worker's job and peak files. 0 means one could not be read, which is never room, so no render may start; held 5 minutes, LaneSignalsUnreadable names it.",
    'min(lane_signal_readable)',
    'none',
    [{ color: 'red', value: null }, { color: 'green', value: 1 }],
    [{ type: 'value', options: { '0': { text: 'one is not' }, '1': { text: 'all' } } }],
  ),

  laneMemory: {
    title: 'Free memory against what a render needs',
    description: "MemAvailable, home-wide, against the floor (what must stay free beyond batch work: 2 GiB) and the floor plus the render's peak (what must be free for a render to start). The peak is the one the render worker measured last, or the declared ceiling (the batch containers' caps less what the speaker keeps between renders) until it measures one.",
    type: 'timeseries',
    datasource: ds,
    targets: [
      { expr: 'max(lane_mem_available_bytes)', legendFormat: 'free (MemAvailable)', refId: 'A' },
      { expr: 'max(lane_floor_bytes)', legendFormat: 'floor', refId: 'B' },
      { expr: 'max(lane_batch_peak_bytes) + max(lane_floor_bytes)', legendFormat: 'to start: peak + floor', refId: 'C' },
      { expr: 'max(lane_declared_peak_bytes) + max(lane_floor_bytes)', legendFormat: 'declared ceiling + floor', refId: 'D' },
    ],
    fieldConfig: {
      defaults: {
        unit: 'bytes',
        min: 0,
        custom: { drawStyle: 'line', lineWidth: 1, fillOpacity: 0, spanNulls: false },
      },
      overrides: [{ matcher: { id: 'byName', options: 'declared ceiling + floor' }, properties: [{ id: 'custom.lineStyle', value: { fill: 'dash', dash: [6, 6] } }] }],
    },
    options: { legend: { displayMode: 'list', placement: 'bottom' }, tooltip: { mode: 'multi' } },
  },

  lanePressure: {
    title: 'Memory pressure against its limit',
    description: "PSI's some avg10 for memory, home-wide: the share of the last 10 s some task stalled waiting for memory. Past the limit (10%) the lane stops the render.",
    type: 'timeseries',
    datasource: ds,
    targets: [
      { expr: 'max(lane_memory_pressure_pct)', legendFormat: 'pressure (some avg10)', refId: 'A' },
      { expr: 'max(lane_pressure_limit_pct)', legendFormat: 'limit', refId: 'B' },
    ],
    fieldConfig: {
      defaults: { unit: 'percent', min: 0, custom: { drawStyle: 'line', lineWidth: 1, fillOpacity: 10, spanNulls: false } },
      overrides: [],
    },
    options: { legend: { displayMode: 'list', placement: 'bottom' }, tooltip: { mode: 'multi' } },
  },

  laneWork: {
    title: 'Who was working',
    description: 'Interactive work (a voice note being heard, a Rust Rounds program being recorded), batch work in progress (a render job named, or an attempt of the call\'s or Playoff Delta\'s running), and the render job the worker named, if any. When none is, home is at rest: WHO DUNNIT\'s AT REST row measures what that costs.',
    type: 'state-timeline',
    datasource: ds,
    targets: [
      { expr: 'max(lane_interactive_waiting)', legendFormat: 'interactive', refId: 'A' },
      { expr: 'max(lane_batch_active)', legendFormat: 'batch', refId: 'B' },
      { expr: 'max(lane_batch_job_present)', legendFormat: 'job named', refId: 'C' },
      { expr: 'max(lane_idle)', legendFormat: 'at rest', refId: 'D' },
    ],
    fieldConfig: {
      defaults: {
        color: { mode: 'thresholds' },
        thresholds: { mode: 'absolute', steps: [{ color: 'transparent', value: null }, { color: 'blue', value: 1 }] },
        mappings: [{ type: 'value', options: { '0': { text: 'no' }, '1': { text: 'yes' } } }],
        custom: { fillOpacity: 70, lineWidth: 0 },
      },
      overrides: [],
    },
    options: { showValue: 'never', mergeValues: true, rowHeight: 0.8, legend: { showLegend: false } },
  },
}
