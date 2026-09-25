// overview-panels.libsonnet
//
// The file_host dashboard's glance layer: the rows between the HEALTH verdicts
// (health-panels.libsonnet) and the drill-down rows below them. Each panel here
// answers one of the operator's standing questions, in the order they get
// asked:
//
//   GOLDEN SIGNALS  what's the traffic, what's the latency, has it gone down,
//                   how long has this build been running
//   SERVICES        are all my services up — and when weren't they
//   METAL           how are my services treating the host (CPU, memory, disk)
//
// Design rules (shared with sympathy-panels.libsonnet via glance.libsonnet),
// so the next panel added here reads like the others:
//
//   - A stat either carries a verdict (thresholds: green / orange / red) or is
//     plainly informational (fixed blue) — never green-by-default, which reads
//     as "healthy" whether or not anything was measured (docs/dashboard-honesty.md).
//   - A stat that can show a trend does (graphMode 'area' over a range query):
//     "how much" and "is it getting worse" in one tile.
//   - Every panel has a `description` saying what it answers and what to do
//     when it isn't green — the (i) on the panel header.
//   - Units read as a person would say them: req/min, not 0.10 req/s.
local panelDefaults = import 'panel-defaults.libsonnet';

local ds = { type: 'prometheus', uid: 'prometheus' };

// The hoist set (infra/prometheus/inventory.yml, `hoist_set: true`) by
// `container_name` — the services this stack runs for its own sake, as
// opposed to the observability stack watching them. cadvisor labels
// containers by that name (`name`).
local services = 'file-host-server|orchestrator-service|some-redis|nats-server|tabsched-ollama';

// The same services by process name (`comm`), for process-exporter — see the
// Disk I/O panel for why that one is read per process rather than per
// container.
local serviceProcesses = 'file_host|orchestrator|redis-server|nats-server|ollama';

// Real filesystems only: tmpfs/overlay/etc. are either memory or a docker
// layer view of a disk already listed, and /nix/store is a read-only bind of /
// on NixOS — the same device a second time.
local realFs = 'fstype!~"tmpfs|overlay|squashfs|ramfs|nsfs|devtmpfs|autofs|fuse.*", mountpoint!~"/nix/store|/var/lib/docker/.+|/run.*"';

local glance = import 'glance.libsonnet';
local steps = glance.steps;
local green = glance.green;
local amber = glance.amber;
local red = glance.red;
local informational = glance.informational;
local sparkStat = glance.sparkStat;
local rangeStat = glance.rangeStat;
local barGauge = glance.barGauge;

{
  // =============== GOLDEN SIGNALS ===============

  traffic: sparkStat(
    'TRAFFIC',
    'Requests per minute actually served to clients: every HTTP route except the /ready and /health probes (and /metrics, which http.rs never counts). Informational, not a verdict: low traffic is not a fault. For "is anyone connected", see WS CLIENTS.',
    'sum(rate(http_requests_total{route!~"/ready|/health"}[5m])) * 60',
    'suffix: req/min',
    informational,
    1,
  ),

  wsClients: sparkStat(
    'WS CLIENTS',
    'Real WebSocket clients connected right now — every client_type except the blackbox probe. Informational. If this disagrees with what you expect, open the Realtime row: LEAKED ENTRIES there says whether the server is counting sockets nobody holds.',
    // `ws_client_connections` has one series per client_type ever seen, so
    // with only the probe ever connected the selector is empty; the fallback
    // reads that as 0 — gated on `ws_connections`, which the periodic sweep
    // always sets, so a missing file_host still reads no data.
    'sum(ws_client_connections{client_type!="probe"}) or on() (0 * max(ws_connections{state="connected"}))',
    'none',
    informational,
    0,
  ),

  latencyP95: sparkStat(
    'LATENCY p95',
    'The 95th-percentile request duration across every route, over 5m windows. Orange from 300ms, red from 1s. If red: the HTTP Latency by Route panel below says which route; a /ws spike is time spent waiting for a connection permit.',
    'histogram_quantile(0.95, sum by (le) (rate(http_request_duration_seconds_bucket[5m])))',
    's',
    steps([{ color: green, value: null }, { color: amber, value: 0.3 }, { color: red, value: 1 }]),
  ),

  availability: rangeStat(
    'AVAILABILITY',
    'Share of scrapes over the selected time range in which Prometheus reached file_host (up{job="file_host"}). Red below 99%, orange below 99.9%. Widen the time picker to ask about a longer window; the SERVICES timeline shows when the misses were.',
    '100 * avg_over_time(up{job="file_host"}[$__range])',
    'percent',
    steps([{ color: red, value: null }, { color: amber, value: 99 }, { color: green, value: 99.9 }]),
    2,
  ),

  restarts: rangeStat(
    'RESTARTS',
    'How many times file_host started over the selected time range. Orange at one or more: a deploy or a crash. Each one is a purple annotation on every graph (the Restarts toggle, top left) so it can be lined up against whatever changed.',
    'changes(process_start_time_seconds{job="file_host"}[$__range])',
    'none',
    steps([{ color: green, value: null }, { color: amber, value: 1 }]),
    0,
  ),

  upFor: sparkStat(
    'UP FOR',
    'Time since this file_host process started. Scoped to job="file_host": every exporter exports process_start_time_seconds too, and an unscoped query rendered one overlapping value per job. The running build is the `build` picker at the top of the dashboard.',
    'time() - max(process_start_time_seconds{job="file_host"})',
    's',
    informational,
    1,
  ) { options+: { graphMode: 'none' }, targets: [{ expr: 'time() - max(process_start_time_seconds{job="file_host"})', instant: true, refId: 'A' }] },

  // =============== SERVICES ===============

  // One lane per service, green while up and red while down, over whatever
  // range the time picker says. A gap (no colour) is "not measured", which
  // the honesty rule keeps distinct from both. Redis and NATS are read
  // through their exporters: `redis_up` is the exporter's own "could I reach
  // redis", and a NATS exporter with no `gnatsd_varz_*` output couldn't
  // reach NATS; an exporter that is itself down reads DOWN rather than a gap,
  // since nothing else will answer for that service.
  serviceTimeline: {
    title: 'SERVICES — up, and when they weren\'t',
    description: 'Green while a service answers, red while it does not, over the selected time range. "file_host /ws" is the blackbox WebSocket handshake — a real client\'s view of /ws, independent of the metrics scrape. Hover a red band for its start and duration.',
    type: 'state-timeline',
    datasource: ds,
    targets: [
      { expr: 'max(up{job="file_host"})', legendFormat: 'file_host', refId: 'A' },
      { expr: 'max(probe_success{job="websocket_blackbox_http"})', legendFormat: 'file_host /ws', refId: 'B' },
      { expr: 'max(up{job="orchestrator"})', legendFormat: 'orchestrator', refId: 'C' },
      { expr: 'max(redis_up) or on() (0 * max(up{job="redis"}))', legendFormat: 'redis', refId: 'D' },
      { expr: '(max(up{job="nats"}) and on() count(gnatsd_varz_connections)) or on() (0 * max(up{job="nats"}))', legendFormat: 'nats', refId: 'E' },
    ],
    fieldConfig: {
      defaults: {
        color: { mode: 'thresholds' },
        thresholds: steps([{ color: red, value: null }, { color: green, value: 1 }]),
        mappings: [{ type: 'value', options: { '0': { text: 'DOWN', color: red }, '1': { text: 'UP', color: green } } }],
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

  // =============== METAL ===============

  cpuVsLimit: barGauge(
    'CPU — % of each service\'s limit',
    'CPU each service is using as a share of its compose `cpus:` limit (cadvisor, 5m average). Past ~90% the kernel starts throttling it: requests queue for CPU even though the host may be idle — see WHO DUNNIT\'s throttling panel. Services without a CPU limit have no bar here.',
    [{
      expr: '100 * sum by (name) (rate(container_cpu_usage_seconds_total{name=~"' + services + '"}[5m])) / on(name) (max by (name) (container_spec_cpu_quota{name=~"' + services + '"}) / max by (name) (container_spec_cpu_period{name=~"' + services + '"}))',
      legendFormat: '{{name}}',
      instant: true,
      refId: 'A',
    }],
    'percent',
    steps([{ color: green, value: null }, { color: amber, value: 70 }, { color: red, value: 90 }]),
    100,
  ),

  memVsLimit: barGauge(
    'Memory — % of each service\'s limit',
    'Working-set memory (what the kernel will not reclaim) as a share of each service\'s compose `mem_limit`. At 100% the kernel OOM-kills it — a RESTARTS tick with no deploy behind it. Services without a limit have no bar here.',
    [{
      expr: '100 * max by (name) (container_memory_working_set_bytes{name=~"' + services + '"}) / max by (name) (container_spec_memory_limit_bytes{name=~"' + services + '"} > 0)',
      legendFormat: '{{name}}',
      instant: true,
      refId: 'A',
    }],
    'percent',
    steps([{ color: green, value: null }, { color: amber, value: 75 }, { color: red, value: 90 }]),
    100,
  ),

  diskIo: barGauge(
    'Disk I/O — per service',
    'Bytes each service process reads plus writes per second, 5m average — storage I/O only (process-exporter, /proc/<pid>/io), so named volumes and the SQLite database count and network traffic does not. Read per process rather than per container because cadvisor has no per-container I/O on a cgroup-v2 host without the io controller delegated. For every other process on the host, see WHO DUNNIT.',
    [{
      expr: 'sum by (groupname) (rate(namedprocess_namegroup_read_bytes_total{groupname=~"' + serviceProcesses + '"}[5m])) + sum by (groupname) (rate(namedprocess_namegroup_write_bytes_total{groupname=~"' + serviceProcesses + '"}[5m]))',
      legendFormat: '{{groupname}}',
      instant: true,
      refId: 'A',
    }],
    'Bps',
    steps([{ color: 'blue', value: null }]),
  ),

  diskSpace: barGauge(
    'Disk space — % used per filesystem',
    'How full each real filesystem on the host is. Orange from 80%, red from 90%: SQLite, Prometheus and container logs all fail writes when their filesystem fills.',
    [{
      expr: 'max by (mountpoint) (100 * (1 - node_filesystem_avail_bytes{' + realFs + '} / node_filesystem_size_bytes{' + realFs + '}))',
      legendFormat: '{{mountpoint}}',
      instant: true,
      refId: 'A',
    }],
    'percent',
    steps([{ color: green, value: null }, { color: amber, value: 80 }, { color: red, value: 90 }]),
    100,
  ),

  // =============== REALTIME ===============

  // The number the old DEVICE CONNS panel was trying to be: store entries
  // with no live socket behind them. `ws_connections{connected}` is the
  // store's size; `ws_client_connections` goes down exactly once per socket
  // at `ConnectionCleanup::drop`. Before that drop also reaped the store
  // entry (websocket.rs, #368), every peer that vanished without a close
  // frame left one behind here, forever.
  leakedEntries: sparkStat(
    'LEAKED ENTRIES',
    'Connection-store entries with no socket behind them: store size minus live sockets. Should sit at 0; anything else means some socket teardown path is not removing its entry (see websocket.rs ConnectionCleanup), and WS CONNS, LIVE and SUBSCRIBED are all inflated by that many.',
    'sum(ws_connections{state="connected"}) - (sum(ws_client_connections) or on() (0 * max(ws_connections{state="connected"})))',
    'none',
    steps([{ color: green, value: null }, { color: red, value: 1 }]),
    0,
  ),

  unknownColor:: panelDefaults.unknownColor,
}
