// sympathy-panels.libsonnet
//
// WHO DUNNIT's glance layer: how the machine is holding up, and who is
// responsible — "mechanical sympathy" for the services and the NixOS host
// they share. Laid out as the USE method (utilisation, saturation, errors),
// one resource per band, so every question has the same three-step shape:
//
//   1. Is the resource busy?            (utilisation — the % tiles)
//   2. Is anything waiting on it?        (saturation — PSI, throttling)
//   3. Who is using it?                  (a ranked bar per process/container)
//
// PSI ("pressure stall information", node_exporter's `pressure` collector) is
// the saturation signal throughout: the share of wall-clock time in which at
// least one task was stalled waiting for CPU, memory or I/O. Utilisation says
// how much of a resource is in use; PSI says whether that use is costing
// anyone anything. A host at 95% CPU with 0% CPU pressure is well used; a
// host at 40% with 30% pressure has a queue.
local glance = import 'glance.libsonnet';
local panelDefaults = import 'panel-defaults.libsonnet';

local steps = glance.steps;
local green = glance.green;
local amber = glance.amber;
local red = glance.red;

// Real filesystems — same exclusions as overview-panels.libsonnet's disk
// space panel (tmpfs/overlay/etc. and NixOS's /nix/store bind of /).
local realFs = 'fstype!~"tmpfs|overlay|squashfs|ramfs|nsfs|devtmpfs|autofs|fuse.*", mountpoint!~"/nix/store|/var/lib/docker/.+|/run.*"';

// Block devices worth reading: not loop devices (snaps, images), RAM disks,
// zram swap, or device-mapper/partition duplicates of a disk already listed.
local realDisk = 'device!~"loop.*|ram.*|zram.*|dm-.*|sr.*"';

// Process names are at most 15 characters (the kernel's `comm`), so they fit
// beside the bar, which leaves room for eight readable rows; container names
// ("file-host-server") don't, so container bars keep glance's names-on-top.
local processBars = { options+: { namePlacement: 'left', sizing: 'auto' } };

local pct(lo, hi) = steps([{ color: green, value: null }, { color: amber, value: lo }, { color: red, value: hi }]);

{
  // =============== VERDICTS ===============

  cpuBusy: glance.sparkStat(
    'CPU',
    'Share of all cores doing work (100% minus idle), 5m average. Orange from 70%, red from 90%. Busy is not a problem on its own — check CPU WAIT next to it.',
    '100 * (1 - avg(rate(node_cpu_seconds_total{mode="idle"}[5m])))',
    'percent',
    pct(70, 90),
    0,
  ),

  cpuPressure: glance.sparkStat(
    'CPU WAIT',
    'PSI: share of time at least one runnable task was waiting for a CPU, 5m average. Orange from 10%, red from 25%. Rising pressure means work is queueing — see who is using the CPU below.',
    '100 * rate(node_pressure_cpu_waiting_seconds_total[5m])',
    'percent',
    pct(10, 25),
    1,
  ),

  memUsed: glance.sparkStat(
    'MEMORY',
    'Memory in use that the kernel cannot simply drop (100% minus MemAvailable). Orange from 80%, red from 90%. Page cache does not count — it is reclaimed on demand.',
    '100 * (1 - node_memory_MemAvailable_bytes / node_memory_MemTotal_bytes)',
    'percent',
    pct(80, 90),
    0,
  ),

  memPressure: glance.sparkStat(
    'MEM WAIT',
    'PSI: share of time at least one task was stalled on memory (reclaim, swap-in, refaults), 5m average. Orange from 5%, red from 20%. This is the early warning before OOM moves.',
    '100 * rate(node_pressure_memory_waiting_seconds_total[5m])',
    'percent',
    pct(5, 20),
    1,
  ),

  diskBusy: glance.sparkStat(
    'DISK',
    'The busiest block device: share of time it had I/O in flight, 5m average. Orange from 60%, red from 90%. For an SSD, busy is not saturated — IO WAIT says whether anyone waited.',
    '100 * max(rate(node_disk_io_time_seconds_total{' + realDisk + '}[5m]))',
    'percent',
    pct(60, 90),
    0,
  ),

  ioPressure: glance.sparkStat(
    'IO WAIT',
    'PSI: share of time at least one task was stalled waiting on block I/O, 5m average. Orange from 10%, red from 30%. High here with low DISK usually means one slow device (e.g. a USB or NTFS mount) rather than a saturated disk.',
    '100 * rate(node_pressure_io_waiting_seconds_total[5m])',
    'percent',
    pct(10, 30),
    1,
  ),

  diskFree: glance.sparkStat(
    'DISK FREE',
    'Free space on the fullest real filesystem. Red under 10%, orange under 20% — SQLite, Prometheus and container logs all fail writes when their filesystem fills. The Disk band below says which filesystem.',
    'min(100 * node_filesystem_avail_bytes{' + realFs + '} / node_filesystem_size_bytes{' + realFs + '})',
    'percent',
    steps([{ color: red, value: null }, { color: amber, value: 10 }, { color: green, value: 20 }]),
    0,
  ),

  oomKills: glance.rangeStat(
    'OOM',
    'Processes the kernel killed for lack of memory over the selected time range (node_vmstat_oom_kill). Any is red: something was killed, and if it was a service, RESTARTS on the file_host dashboard moved too.',
    'increase(node_vmstat_oom_kill[$__range])',
    'none',
    steps([{ color: green, value: null }, { color: red, value: 1 }]),
    0,
  ),

  // =============== COLLECTORS ===============

  collectors: panelDefaults.livenessPanel('Collectors', ['node', 'process_exporter', 'cadvisor'], 0, {}) {
    description: 'Whether Prometheus can scrape each collector this dashboard reads. A DOWN one turns its panels grey ("no data"), not green.',
    // Side by side: stacked, three values in a short tile render illegibly small.
    options+: { orientation: 'vertical' },
  },

  // Process-level CPU is only as honest as process-exporter's accounting.
  // This compares the two independent views of the same quantity: CPU time
  // summed over every process group, against CPU time the host says it
  // spent. Below 100% is normal (kernel threads, which process-exporter's
  // config can't match, and scrape timing); well above it means the process
  // numbers overstate reality and the "who" bars below can't be trusted.
  cpuAccounted: glance.sparkStat(
    'CPU ACCOUNTED',
    'Process CPU (process-exporter, summed over every group) as a share of the CPU the host actually spent (node_exporter). Normally somewhat under 100% — kernel threads are not matched. Red above 120%: the process collector is counting CPU the host never spent, so the per-process bars below overstate it.',
    '100 * sum(rate(namedprocess_namegroup_cpu_seconds_total[5m])) / sum(rate(node_cpu_seconds_total{mode!~"idle|iowait|steal"}[5m]))',
    'percent',
    steps([{ color: green, value: null }, { color: amber, value: 105 }, { color: red, value: 120 }]),
    0,
  ),

  cores: glance.rangeStat(
    'CORES · RAM',
    'The machine these percentages are of: logical CPUs, and total memory.',
    'count(node_cpu_seconds_total{mode="idle"})',
    'none',
    glance.informational,
    0,
  ) {
    targets: [
      { expr: 'count(node_cpu_seconds_total{mode="idle"})', legendFormat: 'cores', instant: true, refId: 'A' },
      { expr: 'node_memory_MemTotal_bytes', legendFormat: 'RAM', instant: true, refId: 'B' },
    ],
    fieldConfig+: { overrides: [{ matcher: { id: 'byName', options: 'RAM' }, properties: [{ id: 'unit', value: 'bytes' }, { id: 'decimals', value: 1 }] }] },
    options+: { textMode: 'value_and_name', orientation: 'vertical' },
  },

  // =============== CPU ===============

  cpuByProcess: glance.barGauge(
    'Who is using the CPU — processes',
    'CPU per process group as a share of the whole machine (all cores), 5m average. Groups are process names (comm, 15 characters — hence ".gnome-shell-wr"). If CPU ACCOUNTED is red, these overstate.',
    [{
      expr: 'topk(8, 100 * sum by (groupname) (rate(namedprocess_namegroup_cpu_seconds_total{groupname=~"$process_group"}[5m])) / scalar(count(node_cpu_seconds_total{mode="idle"})))',
      legendFormat: '{{groupname}}',
      instant: true,
      refId: 'A',
    }],
    'percent',
    pct(25, 50),
    100,
  ) + processBars,

  cpuByContainer: glance.barGauge(
    'Who is hitting their limit — containers',
    'CPU each container is using as a share of its own compose `cpus:` limit. Near 100% it is being throttled (next panel) — its requests wait for CPU even when the host is idle. Containers without a limit are not shown.',
    [{
      expr: 'topk(10, 100 * sum by (name) (rate(container_cpu_usage_seconds_total{name=~"$container"}[5m])) / on(name) (max by (name) (container_spec_cpu_quota{name=~"$container"}) / max by (name) (container_spec_cpu_period{name=~"$container"})))',
      legendFormat: '{{name}}',
      instant: true,
      refId: 'A',
    }],
    'percent',
    pct(70, 90),
    100,
  ),

  cpuThrottling: glance.trend(
    'Throttled — share of CPU periods each container was held back',
    'For every container with a CPU limit: the share of scheduler periods in which it wanted more CPU than its limit and was paused. Anything sustained above a few percent is latency the service pays for its limit, independent of how busy the host is.',
    [{
      expr: '100 * sum by (name) (rate(container_cpu_cfs_throttled_periods_total{name=~"$container"}[5m])) / sum by (name) (rate(container_cpu_cfs_periods_total{name=~"$container"}[5m])) > 0',
      legendFormat: '{{name}}',
      refId: 'A',
    }],
    'percent',
  ),

  cpuTrend: glance.trend(
    'CPU — busy vs waiting',
    'Utilisation (busy) against saturation (PSI: some task waiting for a CPU). The gap between them is the story: busy and not waiting is a well-used machine; waiting at modest busy is a queue.',
    [
      { expr: '100 * (1 - avg(rate(node_cpu_seconds_total{mode="idle"}[5m])))', legendFormat: 'busy %', refId: 'A' },
      { expr: '100 * rate(node_pressure_cpu_waiting_seconds_total[5m])', legendFormat: 'waiting % (PSI)', refId: 'B' },
      { expr: '100 * avg(rate(node_cpu_seconds_total{mode="iowait"}[5m]))', legendFormat: 'iowait %', refId: 'C' },
    ],
    'percent',
  ),

  // =============== MEMORY ===============

  memByProcess: glance.barGauge(
    'Who is using memory — processes',
    'Resident memory (RSS) per process group. Shared pages count once per process, so a group of many processes (browsers) overstates a little.',
    [{
      expr: 'topk(8, sum by (groupname) (namedprocess_namegroup_memory_bytes{memtype="resident", groupname=~"$process_group"}))',
      legendFormat: '{{groupname}}',
      instant: true,
      refId: 'A',
    }],
    'bytes',
    steps([{ color: 'blue', value: null }]),
  ) + processBars,

  memByContainer: glance.barGauge(
    'Who is near their limit — containers',
    'Working-set memory as a share of each container\'s compose `mem_limit`. At 100% the kernel OOM-kills it. Containers without a limit are not shown.',
    [{
      expr: 'topk(10, 100 * max by (name) (container_memory_working_set_bytes{name=~"$container"}) / max by (name) (container_spec_memory_limit_bytes{name=~"$container"} > 0))',
      legendFormat: '{{name}}',
      instant: true,
      refId: 'A',
    }],
    'percent',
    pct(75, 90),
    100,
  ),

  memTrend: glance.trend(
    'Memory — used vs stalled',
    'Used memory (excluding reclaimable cache), swap in use, and memory PSI. Swap climbing while pressure stays flat is harmless parking of cold pages; both climbing together is thrashing.',
    [
      { expr: '100 * (1 - node_memory_MemAvailable_bytes / node_memory_MemTotal_bytes)', legendFormat: 'used %', refId: 'A' },
      { expr: '100 * (1 - node_memory_SwapFree_bytes / (node_memory_SwapTotal_bytes > 0))', legendFormat: 'swap used %', refId: 'B' },
      { expr: '100 * rate(node_pressure_memory_waiting_seconds_total[5m])', legendFormat: 'stalled % (PSI)', refId: 'C' },
    ],
    'percent',
  ),

  // =============== DISK ===============

  diskSpace: glance.barGauge(
    'Disk space — % used per filesystem',
    'How full each real filesystem is. Orange from 80%, red from 90%.',
    [{
      expr: 'max by (mountpoint) (100 * (1 - node_filesystem_avail_bytes{' + realFs + '} / node_filesystem_size_bytes{' + realFs + '}))',
      legendFormat: '{{mountpoint}}',
      instant: true,
      refId: 'A',
    }],
    'percent',
    pct(80, 90),
    100,
  ),

  ioByProcess: glance.barGauge(
    'Who is doing disk I/O — processes',
    'Storage bytes read plus written per second per process group, 5m average (/proc/<pid>/io — network traffic is not included).',
    [{
      expr: 'topk(8, sum by (groupname) (rate(namedprocess_namegroup_read_bytes_total{groupname=~"$process_group"}[5m])) + sum by (groupname) (rate(namedprocess_namegroup_write_bytes_total{groupname=~"$process_group"}[5m])))',
      legendFormat: '{{groupname}}',
      instant: true,
      refId: 'A',
    }],
    'Bps',
    steps([{ color: 'blue', value: null }]),
  ) + processBars,

  diskTrend: glance.trend(
    'Disk — busy, waiting, and how slow',
    'Per device: share of time busy, and the average time an I/O took (await). IO PSI is the host-wide share of time some task waited on I/O. Await climbing on one device is the slow disk; PSI says whether anyone felt it.',
    [
      { expr: '100 * rate(node_disk_io_time_seconds_total{' + realDisk + '}[5m])', legendFormat: '{{device}} busy %', refId: 'A' },
      { expr: '100 * rate(node_pressure_io_waiting_seconds_total[5m])', legendFormat: 'waiting % (PSI)', refId: 'B' },
    ],
    'percent',
  ) {
    targets+: [{
      expr: '(rate(node_disk_read_time_seconds_total{' + realDisk + '}[5m]) + rate(node_disk_write_time_seconds_total{' + realDisk + '}[5m])) / (rate(node_disk_reads_completed_total{' + realDisk + '}[5m]) + rate(node_disk_writes_completed_total{' + realDisk + '}[5m]))',
      legendFormat: '{{device}} await',
      refId: 'C',
    }],
    fieldConfig+: {
      overrides: [{
        matcher: { id: 'byRegexp', options: '.* await' },
        properties: [{ id: 'unit', value: 's' }, { id: 'custom.axisPlacement', value: 'right' }, { id: 'custom.lineStyle', value: { fill: 'dash', dash: [6, 4] } }],
      }],
    },
  },
}
