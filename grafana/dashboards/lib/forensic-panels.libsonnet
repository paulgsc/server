// forensic-panels.libsonnet
// Forensic monitoring panels for "Who Dunnit" system hang detection
// Designed for docker-compose stack with process-exporter and cAdvisor

{
  // ========== CRITICAL INVARIANT VIOLATIONS ==========

  // Primary system hang detector - I/O bound processes causing high load with low CPU
  systemHangDetector:: {
    datasource: 'Prometheus',
    fieldConfig: {
      defaults: {
        mappings: [
          {
            options: {
              '0': { text: 'System Healthy', color: 'green' },
              '1': { text: '🚨 I/O HANG DETECTED!', color: 'red' },
            },
            type: 'value',
          },
        ],
        thresholds: {
          mode: 'absolute',
          steps: [
            { color: 'green', value: null },
            { color: 'red', value: 0.5 },
          ],
        },
        unit: 'none',
      },
      overrides: [],
    },
    options: {
      colorMode: 'background',
      graphMode: 'none',
      justifyMode: 'center',
      orientation: 'auto',
      textMode: 'name',
      textSize: { title: 16, value: 14 },
    },
    targets: [
      {
        datasource: 'Prometheus',
        expr: |||
          (
            (node_load1 / on() (count(count(node_cpu_seconds_total) by (cpu)))) > 3
          )
          and
          (
            (100 - avg(rate(node_cpu_seconds_total{mode="idle"}[5m])) * 100) < 30
          )
        |||,
        instant: true,
        refId: 'A',
      },
    ],
    title: '🚨 SYSTEM HANG DETECTOR',
    description: 'Detects classic I/O-bound hangs: High load average + Low CPU usage',
    type: 'stat',
  },

  // Request rate vs throttling invariant violation
  requestRateInvariant:: {
    datasource: 'Prometheus',
    fieldConfig: {
      defaults: {
        mappings: [
          {
            options: {
              '0': { text: 'Rate Limiting Active', color: 'green' },
              '1': { text: '🚨 NO THROTTLING!', color: 'red' },
            },
            type: 'value',
          },
        ],
        thresholds: {
          mode: 'absolute',
          steps: [
            { color: 'green', value: null },
            { color: 'red', value: 0.5 },
          ],
        },
        unit: 'none',
      },
      overrides: [],
    },
    options: {
      colorMode: 'background',
      graphMode: 'none',
      justifyMode: 'center',
      orientation: 'auto',
      textMode: 'name',
    },
    targets: [
      {
        datasource: 'Prometheus',
        expr: |||
          (
            sum(rate(http_requests_total[1m])) > 20
          )
          and
          (
            sum(rate(http_requests_total{status="429"}[1m])) == 0
          )
        |||,
        instant: true,
        refId: 'A',
      },
    ],
    title: '🚨 High Load + No Rate Limiting',
    description: 'file-host receiving high request rate without 429 responses',
    type: 'stat',
  },

  // Blocked processes in uninterruptible sleep (D state)
  blockedProcesses:: {
    datasource: 'Prometheus',
    fieldConfig: {
      defaults: {
        thresholds: {
          mode: 'absolute',
          steps: [
            { color: 'green', value: null },
            { color: 'yellow', value: 5 },
            { color: 'red', value: 10 },
          ],
        },
        unit: 'none',
      },
      overrides: [],
    },
    options: {
      colorMode: 'value',
      graphMode: 'area',
      justifyMode: 'center',
      orientation: 'auto',
      textMode: 'auto',
    },
    targets: [
      {
        datasource: 'Prometheus',
        expr: 'node_procs_blocked',
        refId: 'A',
      },
    ],
    title: 'Processes in D State',
    description: 'Processes stuck waiting for I/O (uninterruptible sleep)',
    type: 'stat',
  },

  // ========== PRIMARY SUSPECT IDENTIFICATION ==========

  // Top CPU consuming processes - identifies CPU hogs
  topCpuOffenders:: {
    datasource: 'Prometheus',
    fieldConfig: {
      defaults: {
        custom: {
          align: 'auto',
          displayMode: 'auto',
        },
        thresholds: {
          mode: 'absolute',
          steps: [
            { color: 'green', value: null },
            { color: 'yellow', value: 50 },
            { color: 'red', value: 80 },
          ],
        },
        unit: 'percent',
        decimals: 1,
      },
      overrides: [
        {
          matcher: { id: 'byName', options: 'Process Group' },
          properties: [
            { id: 'custom.width', value: 300 },
          ],
        },
        {
          matcher: { id: 'byRegexp', options: '.*file-host.*' },
          properties: [
            { id: 'color', value: { mode: 'fixed', fixedColor: 'blue' } },
          ],
        },
      ],
    },
    options: {
      frameIndex: 0,
      showHeader: true,
      sortBy: [{ desc: true, displayName: 'CPU %' }],
    },
    targets: [
      {
        datasource: 'Prometheus',
        expr: 'topk(15, sum by (groupname) (rate(namedprocess_namegroup_cpu_seconds_total[1m])) * 100)',
        format: 'table',
        instant: true,
        refId: 'A',
      },
    ],
    title: '👤 TOP CPU OFFENDERS (% of 1 core)',
    description: 'Processes consuming most CPU - file-host highlighted in blue',
    transformations: [
      {
        id: 'organize',
        options: {
          excludeByName: {
            Time: true,
            __name__: true,
            job: true,
            instance: true,
          },
          renameByName: {
            groupname: 'Process Group',
            Value: 'CPU %',
          },
        },
      },
    ],
    type: 'table',
  },

  // Top memory consuming processes - leak detection
  topMemoryConsumers:: {
    datasource: 'Prometheus',
    fieldConfig: {
      defaults: {
        custom: {
          align: 'auto',
          displayMode: 'auto',
        },
        unit: 'bytes',
      },
      overrides: [
        {
          matcher: { id: 'byName', options: 'Process Group' },
          properties: [
            { id: 'custom.width', value: 300 },
          ],
        },
        {
          matcher: { id: 'byRegexp', options: '.*metabase.*' },
          properties: [
            { id: 'color', value: { mode: 'fixed', fixedColor: 'orange' } },
          ],
        },
      ],
    },
    options: {
      frameIndex: 0,
      showHeader: true,
      sortBy: [{ desc: true, displayName: 'Memory (RSS)' }],
    },
    targets: [
      {
        datasource: 'Prometheus',
        expr: 'topk(15, sum by (groupname) (namedprocess_namegroup_memory_bytes{memtype="resident"}))',
        format: 'table',
        instant: true,
        refId: 'A',
      },
    ],
    title: '🧠 TOP MEMORY CONSUMERS',
    description: 'Processes using most RAM - metabase highlighted in orange',
    transformations: [
      {
        id: 'organize',
        options: {
          excludeByName: {
            Time: true,
            __name__: true,
            job: true,
            instance: true,
            memtype: true,
          },
          renameByName: {
            groupname: 'Process Group',
            Value: 'Memory (RSS)',
          },
        },
      },
    ],
    type: 'table',
  },

  // Top I/O bandwidth consumers - disk thrashing culprits
  topIoOffenders:: {
    datasource: 'Prometheus',
    fieldConfig: {
      defaults: {
        custom: {
          align: 'auto',
          displayMode: 'auto',
        },
        unit: 'Bps',
      },
      overrides: [
        {
          matcher: { id: 'byName', options: 'Process Group' },
          properties: [
            { id: 'custom.width', value: 300 },
          ],
        },
      ],
    },
    options: {
      frameIndex: 0,
      showHeader: true,
      sortBy: [{ desc: true, displayName: 'Total I/O Rate' }],
    },
    targets: [
      {
        datasource: 'Prometheus',
        expr: 'topk(10, sum by (groupname) (rate(namedprocess_namegroup_read_bytes_total[1m]) + rate(namedprocess_namegroup_write_bytes_total[1m])))',
        format: 'table',
        instant: true,
        refId: 'A',
      },
    ],
    title: '💾 TOP I/O BANDWIDTH CRIMINALS',
    description: 'Processes doing the most disk reads/writes',
    transformations: [
      {
        id: 'organize',
        options: {
          excludeByName: {
            Time: true,
            __name__: true,
            job: true,
            instance: true,
          },
          renameByName: {
            groupname: 'Process Group',
            Value: 'Total I/O Rate',
          },
        },
      },
    ],
    type: 'table',
  },

  // ========== SYSTEM CORRELATION MATRIX ==========

  // Load average vs actual CPU usage - primary hang indicator
  loadVsCpuDivergence:: {
    datasource: 'Prometheus',
    fieldConfig: {
      defaults: {
        custom: {
          thresholdsStyle: { mode: 'line' },
        },
        unit: 'none',
        min: 0,
      },
      overrides: [
        {
          matcher: { id: 'byName', options: 'Load per Core' },
          properties: [
            { id: 'color', value: { mode: 'fixed', fixedColor: 'red' } },
            { id: 'custom.lineWidth', value: 3 },
          ],
        },
        {
          matcher: { id: 'byName', options: 'CPU Usage %' },
          properties: [
            { id: 'color', value: { mode: 'fixed', fixedColor: 'blue' } },
            { id: 'custom.lineWidth', value: 2 },
          ],
        },
      ],
    },
    options: {
      legend: { displayMode: 'visible', placement: 'bottom' },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
    targets: [
      {
        datasource: 'Prometheus',
        expr: 'node_load1 / on() (count(count(node_cpu_seconds_total) by (cpu)))',
        legendFormat: 'Load per Core',
        refId: 'A',
      },
      {
        datasource: 'Prometheus',
        expr: '100 - avg(rate(node_cpu_seconds_total{mode="idle"}[5m])) * 100',
        legendFormat: 'CPU Usage %',
        refId: 'B',
      },
    ],
    title: '📊 LOAD vs CPU DIVERGENCE',
    description: 'When load is high but CPU low = I/O bound processes causing hangs',
    type: 'timeseries',
  },

  // Process state distribution - shows blocked/zombie processes
  processStates:: {
    datasource: 'Prometheus',
    fieldConfig: {
      defaults: {
        custom: {
          stacking: { mode: 'normal', group: 'A' },
        },
        unit: 'none',
      },
      overrides: [
        {
          matcher: { id: 'byRegexp', options: '.*D.*|.*Blocked.*' },
          properties: [
            { id: 'color', value: { mode: 'fixed', fixedColor: 'red' } },
          ],
        },
        {
          matcher: { id: 'byRegexp', options: '.*Z.*|.*Zombie.*' },
          properties: [
            { id: 'color', value: { mode: 'fixed', fixedColor: 'orange' } },
          ],
        },
        {
          matcher: { id: 'byRegexp', options: '.*R.*|.*Running.*' },
          properties: [
            { id: 'color', value: { mode: 'fixed', fixedColor: 'green' } },
          ],
        },
      ],
    },
    options: {
      legend: { displayMode: 'visible', placement: 'right' },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
    targets: [
      {
        datasource: 'Prometheus',
        expr: 'sum by (state) (namedprocess_namegroup_states)',
        legendFormat: 'State: {{state}}',
        refId: 'A',
      },
    ],
    title: '🔄 PROCESS STATES',
    description: 'D=Blocked (red), Z=Zombie (orange), R=Running (green)',
    type: 'timeseries',
  },

  // ========== CONTAINER FORENSICS ==========

  // Container CPU usage - identifies container-level CPU hogs
  containerCpuUsage:: {
    datasource: 'Prometheus',
    fieldConfig: {
      defaults: {
        custom: {},
        unit: 'none',
      },
      overrides: [
        {
          matcher: { id: 'byRegexp', options: '.*file-host.*' },
          properties: [
            { id: 'color', value: { mode: 'fixed', fixedColor: 'blue' } },
            { id: 'custom.lineWidth', value: 3 },
          ],
        },
        {
          matcher: { id: 'byRegexp', options: '.*metabase.*' },
          properties: [
            { id: 'color', value: { mode: 'fixed', fixedColor: 'orange' } },
          ],
        },
        {
          matcher: { id: 'byRegexp', options: '.*grafana.*' },
          properties: [
            { id: 'color', value: { mode: 'fixed', fixedColor: 'purple' } },
          ],
        },
      ],
    },
    options: {
      legend: { displayMode: 'visible', placement: 'right' },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
    targets: [
      {
        datasource: 'Prometheus',
        expr: 'topk(8, sum by (name) (rate(container_cpu_usage_seconds_total{name!=""}[1m])))',
        legendFormat: '{{name}}',
        refId: 'A',
      },
    ],
    title: '🐳 CONTAINER CPU USAGE (Cores)',
    description: 'CPU cores used by each container - file-host limit: 2.0',
    type: 'timeseries',
  },

  // Container memory pressure vs limits
  containerMemoryPressure:: {
    datasource: 'Prometheus',
    fieldConfig: {
      defaults: {
        custom: {
          thresholdsStyle: { mode: 'line' },
        },
        unit: 'percent',
        max: 100,
        thresholds: {
          mode: 'absolute',
          steps: [
            { color: 'green', value: null },
            { color: 'yellow', value: 80 },
            { color: 'red', value: 95 },
          ],
        },
      },
      overrides: [],
    },
    options: {
      legend: { displayMode: 'visible', placement: 'right' },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
    targets: [
      {
        datasource: 'Prometheus',
        expr: |||
          (
            container_memory_usage_bytes{name!=""}
            /
            on(name) container_spec_memory_limit_bytes{name!=""}
          ) * 100
        |||,
        legendFormat: '{{name}}',
        refId: 'A',
      },
    ],
    title: '🐳 CONTAINER MEMORY PRESSURE (%)',
    description: 'Memory usage vs container limits - file-host limit: 4GB',
    type: 'timeseries',
  },

  // Container I/O bandwidth
  containerIoBandwidth:: {
    datasource: 'Prometheus',
    fieldConfig: {
      defaults: {
        custom: {},
        unit: 'Bps',
      },
      overrides: [],
    },
    options: {
      legend: { displayMode: 'visible', placement: 'right' },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
    targets: [
      {
        datasource: 'Prometheus',
        expr: |||
          topk(6, sum by (name) (
            rate(container_fs_reads_bytes_total{name!=""}[1m]) + 
            rate(container_fs_writes_bytes_total{name!=""}[1m])
          ))
        |||,
        legendFormat: '{{name}}',
        refId: 'A',
      },
    ],
    title: '🐳 CONTAINER I/O BANDWIDTH',
    description: 'Container filesystem read + write rates',
    type: 'timeseries',
  },

  // ========== FORENSIC EVIDENCE COLLECTION ==========

  // Context switching storm detection
  contextSwitches:: {
    datasource: 'Prometheus',
    fieldConfig: {
      defaults: {
        custom: {},
        unit: 'ops',
      },
      overrides: [
        {
          matcher: { id: 'byName', options: 'System Total' },
          properties: [
            { id: 'color', value: { mode: 'fixed', fixedColor: 'red' } },
            { id: 'custom.lineWidth', value: 3 },
          ],
        },
      ],
    },
    options: {
      legend: { displayMode: 'visible', placement: 'right' },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
    targets: [
      {
        datasource: 'Prometheus',
        expr: 'topk(6, sum by (groupname) (rate(namedprocess_namegroup_context_switches_total{ctxswitchtype="nonvoluntary"}[1m])))',
        legendFormat: '{{groupname}} (involuntary)',
        refId: 'A',
      },
      {
        datasource: 'Prometheus',
        expr: 'rate(node_context_switches_total[1m])',
        legendFormat: 'System Total',
        refId: 'B',
      },
    ],
    title: '⚡ CONTEXT SWITCHES STORM',
    description: 'Involuntary context switches indicate scheduler contention',
    type: 'timeseries',
  },

  // Major page faults - swap thrashing detection
  majorPageFaults:: {
    datasource: 'Prometheus',
    fieldConfig: {
      defaults: {
        custom: {},
        unit: 'ops',
        thresholds: {
          mode: 'absolute',
          steps: [
            { color: 'green', value: null },
            { color: 'yellow', value: 1 },
            { color: 'red', value: 10 },
          ],
        },
      },
      overrides: [],
    },
    options: {
      legend: { displayMode: 'visible', placement: 'right' },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
    targets: [
      {
        datasource: 'Prometheus',
        expr: 'topk(8, sum by (groupname) (rate(namedprocess_namegroup_major_page_faults_total[1m])))',
        legendFormat: '{{groupname}}',
        refId: 'A',
      },
    ],
    title: '💔 MAJOR PAGE FAULTS',
    description: 'Page faults requiring disk I/O - indicates memory pressure',
    type: 'timeseries',
  },

  // Thread count explosion detection
  threadExplosion:: {
    datasource: 'Prometheus',
    fieldConfig: {
      defaults: {
        custom: {},
        unit: 'none',
        thresholds: {
          mode: 'absolute',
          steps: [
            { color: 'green', value: null },
            { color: 'yellow', value: 50 },
            { color: 'red', value: 100 },
          ],
        },
      },
      overrides: [
        {
          matcher: { id: 'byRegexp', options: '.*file-host.*' },
          properties: [
            { id: 'color', value: { mode: 'fixed', fixedColor: 'blue' } },
            { id: 'custom.lineWidth', value: 2 },
          ],
        },
      ],
    },
    options: {
      legend: { displayMode: 'visible', placement: 'right' },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
    targets: [
      {
        datasource: 'Prometheus',
        expr: 'topk(10, sum by (groupname) (namedprocess_namegroup_num_threads))',
        legendFormat: '{{groupname}}',
        refId: 'A',
      },
    ],
    title: '🧵 THREAD COUNT',
    description: 'Thread explosion detection - file-host uses Tokio threads',
    type: 'timeseries',
  },

  // File descriptor usage and leaks
  fileDescriptorUsage:: {
    datasource: 'Prometheus',
    fieldConfig: {
      defaults: {
        custom: {},
        unit: 'percentunit',
        max: 1,
        thresholds: {
          mode: 'absolute',
          steps: [
            { color: 'green', value: null },
            { color: 'yellow', value: 0.7 },
            { color: 'red', value: 0.9 },
          ],
        },
      },
      overrides: [],
    },
    options: {
      legend: { displayMode: 'visible', placement: 'right' },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
    targets: [
      {
        datasource: 'Prometheus',
        expr: 'topk(10, max by (groupname) (namedprocess_namegroup_worst_fd_ratio))',
        legendFormat: '{{groupname}}',
        refId: 'A',
      },
    ],
    title: '📁 FILE DESCRIPTOR USAGE RATIO',
    description: 'FD exhaustion risk - ratio of used/available file descriptors',
    type: 'timeseries',
  },

  // ========== SYSTEM-WIDE HEALTH INDICATORS ==========

  // Disk I/O queue depth and latency
  diskIoQueue:: {
    datasource: 'Prometheus',
    fieldConfig: {
      defaults: {
        custom: {},
        unit: 'none',
        thresholds: {
          mode: 'absolute',
          steps: [
            { color: 'green', value: null },
            { color: 'yellow', value: 5 },
            { color: 'red', value: 10 },
          ],
        },
      },
      overrides: [
        {
          matcher: { id: 'byName', options: 'Current Queue Depth' },
          properties: [
            { id: 'color', value: { mode: 'fixed', fixedColor: 'orange' } },
          ],
        },
      ],
    },
    options: {
      legend: { displayMode: 'visible', placement: 'bottom' },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
    targets: [
      {
        datasource: 'Prometheus',
        expr: 'avg_over_time(node_disk_io_now[5m])',
        legendFormat: 'Avg Queue Depth (5m)',
        refId: 'A',
      },
      {
        datasource: 'Prometheus',
        expr: 'node_disk_io_now',
        legendFormat: 'Current Queue Depth',
        refId: 'B',
      },
    ],
    title: '💿 DISK I/O QUEUE DEPTH',
    description: 'I/O operations waiting for disk - queue >10 indicates bottleneck',
    type: 'timeseries',
  },

  // Swap usage and thrashing indicators
  swapThrashing:: {
    datasource: 'Prometheus',
    fieldConfig: {
      defaults: {
        custom: {},
        unit: 'percent',
      },
      overrides: [
        {
          matcher: { id: 'byName', options: 'Swap Usage %' },
          properties: [
            {
              id: 'thresholds',
              value: {
                mode: 'absolute',
                steps: [
                  { color: 'green', value: null },
                  { color: 'yellow', value: 5 },
                  { color: 'red', value: 20 },
                ],
              },
            },
          ],
        },
      ],
    },
    options: {
      legend: { displayMode: 'visible', placement: 'bottom' },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
    targets: [
      {
        datasource: 'Prometheus',
        expr: '(node_memory_SwapTotal_bytes - node_memory_SwapFree_bytes) / node_memory_SwapTotal_bytes * 100',
        legendFormat: 'Swap Usage %',
        refId: 'A',
      },
      {
        datasource: 'Prometheus',
        expr: 'rate(node_vmstat_pgmajfault[1m])',
        legendFormat: 'Major Page Faults/sec',
        refId: 'B',
      },
    ],
    title: '🔄 SWAP USAGE & THRASHING',
    description: 'Swap usage >5% or page faults >1/sec indicates memory pressure',
    type: 'timeseries',
  },

  // Docker-compose specific container process limits
  containerProcessLimits:: {
    datasource: 'Prometheus',
    fieldConfig: {
      defaults: {
        custom: {
          align: 'auto',
          displayMode: 'auto',
        },
        unit: 'none',
        thresholds: {
          mode: 'absolute',
          steps: [
            { color: 'green', value: null },
            { color: 'yellow', value: 150 },  // file-host pids_limit: 200
            { color: 'red', value: 180 },
          ],
        },
      },
      overrides: [],
    },
    options: {
      frameIndex: 0,
      showHeader: true,
      sortBy: [{ desc: true, displayName: 'Process Count' }],
    },
    targets: [
      {
        datasource: 'Prometheus',
        expr: 'container_processes{name!=""}',
        format: 'table',
        instant: true,
        refId: 'A',
      },
    ],
    title: '🐳 CONTAINER PROCESS COUNTS',
    description: 'Current vs pids_limit - file-host: 200, metabase: 100',
    transformations: [
      {
        id: 'organize',
        options: {
          excludeByName: {
            Time: true,
            __name__: true,
            job: true,
            instance: true,
            id: true,
            image: true,
          },
          renameByName: {
            name: 'Container Name',
            Value: 'Process Count',
          },
        },
      },
    ],
    type: 'table',
  },
}
