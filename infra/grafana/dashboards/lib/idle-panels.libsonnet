// idle-panels.libsonnet
//
// WHO DUNNIT's AT REST row: what home's services cost while nothing is being
// worked on, its idle footprint. "At rest" is file_host's batch lane's
// `lane_idle` (apps/servers/file_host/src/call_weeks/lane.rs): no
// interactive work held in its 5 s tick (no voice note being heard, no Rust
// Rounds program being recorded) and no batch work in progress (no render
// job named, no week's attempt running). Everything else keeps running at
// rest, and is counted: the polls (mailbox, call weeks, game feed, the nudge
// waker), requests, and every container staying up. That is the cost the
// row measures.
//
// Each panel keeps a series only at the moments the lane said rest
// (`and on() (max(lane_idle) == 1)`), then averages over the dashboard's
// range, one sample a minute. No data: the lane has not seen home at rest in
// the range (or file_host was not scraped).
local glance = import 'glance.libsonnet';

local steps = glance.steps;

// Only while the lane says home is at rest.
local atRest(expr) = '(' + expr + ') and on() (max(lane_idle) == 1)';
local averaged(expr) = 'avg_over_time((' + atRest(expr) + ')[$__range:1m])';

local hostUsed = 'node_memory_MemTotal_bytes - node_memory_MemAvailable_bytes';
local hostCores = 'sum(rate(node_cpu_seconds_total{mode!~"idle|iowait|steal"}[2m]))';

{
  share: glance.rangeStat(
    'TIME AT REST',
    "Share of the range in which nothing was being worked on: no voice note heard, no Rust Rounds program recorded, no render in progress (file_host's batch lane, lane_idle). The rest of the row is measured in this time only.",
    '100 * avg_over_time(max(lane_idle)[$__range:1m])',
    'percent',
    glance.informational,
    0,
  ),

  memory: glance.rangeStat(
    'MEMORY AT REST',
    "Home's memory in use (all but MemAvailable) while at rest, averaged over the range: what keeping every service up costs with nothing to do. A render's peak and the lane's floor have to fit beside it.",
    averaged(hostUsed),
    'bytes',
    glance.informational,
    1,
  ),

  cores: glance.rangeStat(
    'CORES BUSY AT REST',
    "Cores' worth of CPU home's host spends while at rest, averaged over the range: the polls, scrapes and requests that run whatever else does.",
    averaged(hostCores),
    'none',
    glance.informational,
    2,
  ),

  memoryByContainer: glance.barGauge(
    'Memory at rest, by container',
    "Each container's working set (what the kernel will not reclaim) while home is at rest, averaged over the range. A batch service that holds memory between jobs (the speaker keeps its model, about 1.25 GB) is counted here, since it stays up.",
    [{ expr: 'sort_desc(' + averaged('max by (name) (container_memory_working_set_bytes{name!=""})') + ')', legendFormat: '{{name}}', instant: true, refId: 'A' }],
    'bytes',
    glance.informational,
  ),

  cpuByContainer: glance.barGauge(
    'CPU at rest, by container',
    "Each container's CPU while home is at rest, in cores, averaged over the range: who spends the CPU when there is nothing to do.",
    [{ expr: 'sort_desc(' + averaged('sum by (name) (rate(container_cpu_usage_seconds_total{name!=""}[2m]))') + ')', legendFormat: '{{name}}', instant: true, refId: 'A' }],
    'none',
    glance.informational,
  ) { fieldConfig+: { defaults+: { decimals: 3 } } },

  trend: glance.trend(
    "Home's memory in use, and at rest",
    'In use is all of home\'s memory but MemAvailable, at every moment; at rest is the same line drawn only while nothing is being worked on. Where the two meet is the baseline every job starts from.',
    [
      { expr: hostUsed, legendFormat: 'in use', refId: 'A' },
      { expr: atRest(hostUsed), legendFormat: 'at rest', refId: 'B' },
    ],
    'bytes',
  ),
}
