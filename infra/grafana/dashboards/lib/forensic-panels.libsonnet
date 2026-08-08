// forensic-panels.libsonnet
// Forensic monitoring panels for "Who Dunnit" system hang detection
// Designed for docker-compose stack with process-exporter and cAdvisor

{
  // ========== CRITICAL INVARIANT VIOLATIONS ==========

  // ✅ FIXED: System Hang Detector — High Load + Low CPU
  systemHangDetector:: {
    datasource: { type: 'prometheus', uid: 'prometheus' },
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
      textMode: 'value_and_name',
      textSize: { title: 16, value: 14 },
    },
    targets: [
      {
        datasource: { type: 'prometheus', uid: 'prometheus' },
        expr: |||
          (
            (node_load1 / count without(cpu, mode) (node_cpu_seconds_total{mode="idle"})) > 3
          )
          and
          (
            (100 - avg without(cpu, mode) (rate(node_cpu_seconds_total{mode="idle"}[5m])) * 100) < 30
          )
        |||,
        instant: true,
        refId: 'A',
      },
    ],
    title: '🚨 SYSTEM HANG DETECTOR',
    description: 'High load average + Low CPU usage = I/O bound hang',
    type: 'stat',
  },

  // #212/#217: `requestRateInvariant` used to live here — a contradiction
  // panel against `http_requests_total`, a name nothing in this workspace
  // emits (no axum-prometheus layer, no manual counter). Same idea, same
  // missing premise as the parked `rate-limit.jsonnet`; removed rather than
  // parked since this file's other 17 panels are real and it has no home
  // of its own to park into. Returns once [ABUSE] defines a real counter.

  // ✅ FIXED: Blocked Processes (D state) — node_procs_blocked is correct
  blockedProcesses:: {
    datasource: { type: 'prometheus', uid: 'prometheus' },
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
        datasource: { type: 'prometheus', uid: 'prometheus' },
        expr: 'node_procs_blocked',
        refId: 'A',
      },
    ],
    title: 'Processes in D State',
    description: 'Processes stuck in uninterruptible sleep (I/O wait)',
    type: 'stat',
  },

  // ========== PRIMARY SUSPECT IDENTIFICATION ==========

  // ✅ FIXED: Top CPU Offenders — process-exporter metric corrected
  topCpuOffenders:: {
    datasource: { type: 'prometheus', uid: 'prometheus' },
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
        unit: 'percentunit',  // % of single core
        decimals: 1,
      },
      overrides: [
        {
          matcher: { id: 'byName', options: 'Process Group' },
          properties: [{ id: 'custom.width', value: 300 }],
        },
        {
          matcher: { id: 'byRegexp', options: '.*file-host.*' },
          properties: [{ id: 'color', value: { mode: 'fixed', fixedColor: 'blue' } }],
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
        datasource: { type: 'prometheus', uid: 'prometheus' },
        expr: |||
          topk(15,
            sum by (groupname) (
              rate(namedprocess_namegroup_cpu_seconds_total{groupname!=""}[1m])
            ) * 100
          )
        |||,
        format: 'table',
        instant: true,
        refId: 'A',
      },
    ],
    title: '👤 TOP CPU OFFENDERS (% of 1 core)',
    description: 'Processes consuming most CPU — file-host in blue',
    transformations: [
      {
        id: 'organize',
        options: {
          excludeByName: { Time: true, __name__: true, job: true, instance: true },
          renameByName: { groupname: 'Process Group', Value: 'CPU %' },
        },
      },
    ],
    type: 'table',
  },

  // ✅ FIXED: Top Memory Consumers — RSS memory is correct
  topMemoryConsumers:: {
    datasource: { type: 'prometheus', uid: 'prometheus' },
    fieldConfig: {
      defaults: {
        custom: { align: 'auto', displayMode: 'auto' },
        unit: 'bytes',
      },
      overrides: [
        {
          matcher: { id: 'byName', options: 'Process Group' },
          properties: [{ id: 'custom.width', value: 300 }],
        },
        {
          matcher: { id: 'byRegexp', options: '.*metabase.*' },
          properties: [{ id: 'color', value: { mode: 'fixed', fixedColor: 'orange' } }],
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
        datasource: { type: 'prometheus', uid: 'prometheus' },
        expr: |||
          topk(15,
            sum by (groupname) (
              namedprocess_namegroup_memory_bytes{memtype="resident", groupname!=""}
            )
          )
        |||,
        format: 'table',
        instant: true,
        refId: 'A',
      },
    ],
    title: '🧠 TOP MEMORY CONSUMERS',
    description: 'Processes using most RAM — metabase in orange',
    transformations: [
      {
        id: 'organize',
        options: {
          excludeByName: { Time: true, __name__: true, job: true, instance: true, memtype: true },
          renameByName: { groupname: 'Process Group', Value: 'Memory (RSS)' },
        },
      },
    ],
    type: 'table',
  },

  // ✅ FIXED: Top I/O Bandwidth — read + write bytes per second
  topIoOffenders:: {
    datasource: { type: 'prometheus', uid: 'prometheus' },
    fieldConfig: {
      defaults: {
        custom: { align: 'auto', displayMode: 'auto' },
        unit: 'Bps',
      },
      overrides: [
        {
          matcher: { id: 'byName', options: 'Process Group' },
          properties: [{ id: 'custom.width', value: 300 }],
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
        datasource: { type: 'prometheus', uid: 'prometheus' },
        expr: |||
          topk(10,
            sum by (groupname) (
              rate(namedprocess_namegroup_read_bytes_total{groupname!=""}[1m]) +
              rate(namedprocess_namegroup_write_bytes_total{groupname!=""}[1m])
            )
          )
        |||,
        format: 'table',
        instant: true,
        refId: 'A',
      },
    ],
    title: '💾 TOP I/O BANDWIDTH CRIMINALS',
    description: 'Processes doing most disk I/O',
    transformations: [
      {
        id: 'organize',
        options: {
          excludeByName: { Time: true, __name__: true, job: true, instance: true },
          renameByName: { groupname: 'Process Group', Value: 'Total I/O Rate' },
        },
      },
    ],
    type: 'table',
  },

  // ========== SYSTEM CORRELATION MATRIX ==========

  // ✅ FIXED: Load vs CPU — per-core normalization corrected
  loadVsCpuDivergence:: {
    datasource: { type: 'prometheus', uid: 'prometheus' },
    fieldConfig: {
      defaults: {
        custom: { thresholdsStyle: { mode: 'line' } },
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
        datasource: { type: 'prometheus', uid: 'prometheus' },
        expr: 'node_load1 / count without(cpu, mode) (node_cpu_seconds_total{mode="idle"})',
        legendFormat: 'Load per Core',
        refId: 'A',
      },
      {
        datasource: { type: 'prometheus', uid: 'prometheus' },
        expr: '100 - (avg without(cpu, mode) (rate(node_cpu_seconds_total{mode="idle"}[5m])) * 100)',
        legendFormat: 'CPU Usage %',
        refId: 'B',
      },
    ],
    title: '📊 LOAD vs CPU DIVERGENCE',
    description: 'High load + low CPU = I/O bound bottleneck',
    type: 'timeseries',
  },

  // ✅ FIXED: Process States — state labels: R, S, D, Z, etc.
  processStates:: {
    datasource: { type: 'prometheus', uid: 'prometheus' },
    fieldConfig: {
      defaults: {
        custom: { stacking: { mode: 'normal', group: 'A' } },
        unit: 'none',
      },
      overrides: [
        {
          matcher: { id: 'byRegexp', options: '.*D.*' },
          properties: [{ id: 'color', value: { mode: 'fixed', fixedColor: 'red' } }],
        },
        {
          matcher: { id: 'byRegexp', options: '.*Z.*' },
          properties: [{ id: 'color', value: { mode: 'fixed', fixedColor: 'orange' } }],
        },
        {
          matcher: { id: 'byRegexp', options: '.*R.*' },
          properties: [{ id: 'color', value: { mode: 'fixed', fixedColor: 'green' } }],
        },
      ],
    },
    options: {
      legend: { displayMode: 'visible', placement: 'right' },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
    targets: [
      {
        datasource: { type: 'prometheus', uid: 'prometheus' },
        expr: 'sum by (state) (namedprocess_namegroup_states{state!=""})',
        legendFormat: 'State: {{state}}',
        refId: 'A',
      },
    ],
    title: '🔄 PROCESS STATES',
    description: 'D=Blocked (red), Z=Zombie (orange), R=Running (green)',
    type: 'timeseries',
  },

  // ========== CONTAINER FORENSICS ==========

  // ✅ FIXED: Container CPU Usage — cAdvisor metric
  containerCpuUsage:: {
    datasource: { type: 'prometheus', uid: 'prometheus' },
    fieldConfig: {
      defaults: { custom: {}, unit: 'none' },
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
          properties: [{ id: 'color', value: { mode: 'fixed', fixedColor: 'orange' } }],
        },
        {
          matcher: { id: 'byRegexp', options: '.*grafana.*' },
          properties: [{ id: 'color', value: { mode: 'fixed', fixedColor: 'purple' } }],
        },
      ],
    },
    options: {
      legend: { displayMode: 'visible', placement: 'right' },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
    targets: [
      {
        datasource: { type: 'prometheus', uid: 'prometheus' },
        expr: |||
          topk(8,
            sum by (name) (
              rate(container_cpu_usage_seconds_total{name!="", container_label_com_docker_compose_service!=""}[1m])
            )
          )
        |||,
        legendFormat: '{{container_label_com_docker_compose_service}}',
        refId: 'A',
      },
    ],
    title: '🐳 CONTAINER CPU USAGE (Cores)',
    description: 'CPU cores used — file-host limit: 2.0',
    type: 'timeseries',
  },

  // ✅ FIXED: Container Memory Pressure — usage vs limit
  containerMemoryPressure:: {
    datasource: { type: 'prometheus', uid: 'prometheus' },
    fieldConfig: {
      defaults: {
        custom: { thresholdsStyle: { mode: 'line' } },
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
        datasource: { type: 'prometheus', uid: 'prometheus' },
        expr: |||
          (
            container_memory_usage_bytes{name!="", container_label_com_docker_compose_service!=""}
            /
            container_spec_memory_limit_bytes{name!="", container_label_com_docker_compose_service!=""}
          ) * 100
        |||,
        legendFormat: '{{container_label_com_docker_compose_service}}',
        refId: 'A',
      },
    ],
    title: '🐳 CONTAINER MEMORY PRESSURE (%)',
    description: 'Memory usage vs limit — file-host limit: 4GB',
    type: 'timeseries',
  },

  // ✅ FIXED: Container I/O Bandwidth — cAdvisor fs metrics
  containerIoBandwidth:: {
    datasource: { type: 'prometheus', uid: 'prometheus' },
    fieldConfig: {
      defaults: { custom: {}, unit: 'Bps' },
      overrides: [],
    },
    options: {
      legend: { displayMode: 'visible', placement: 'right' },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
    targets: [
      {
        datasource: { type: 'prometheus', uid: 'prometheus' },
        expr: |||
          topk(6,
            sum by (name) (
              rate(container_fs_reads_bytes_total{name!="", container_label_com_docker_compose_service!=""}[1m]) +
              rate(container_fs_writes_bytes_total{name!="", container_label_com_docker_compose_service!=""}[1m])
            )
          )
        |||,
        legendFormat: '{{container_label_com_docker_compose_service}}',
        refId: 'A',
      },
    ],
    title: '🐳 CONTAINER I/O BANDWIDTH',
    description: 'Container filesystem I/O rate',
    type: 'timeseries',
  },

  // ========== FORENSIC EVIDENCE COLLECTION ==========

  // ✅ FIXED: Context Switches — involuntary switches indicate contention
  contextSwitches:: {
    datasource: { type: 'prometheus', uid: 'prometheus' },
    fieldConfig: {
      defaults: { custom: {}, unit: 'ops' },
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
        datasource: { type: 'prometheus', uid: 'prometheus' },
        expr: |||
          topk(6,
            sum by (groupname) (
              rate(namedprocess_namegroup_context_switches_total{ctxswitchtype="nonvoluntary", groupname!=""}[1m])
            )
          )
        |||,
        legendFormat: '{{groupname}} (involuntary)',
        refId: 'A',
      },
      {
        datasource: { type: 'prometheus', uid: 'prometheus' },
        expr: 'rate(node_context_switches_total[1m])',
        legendFormat: 'System Total',
        refId: 'B',
      },
    ],
    title: '⚡ CONTEXT SWITCHES STORM',
    description: 'High involuntary switches = CPU scheduling pressure',
    type: 'timeseries',
  },

  // ✅ FIXED: Major Page Faults — disk-backed page faults
  majorPageFaults:: {
    datasource: { type: 'prometheus', uid: 'prometheus' },
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
        datasource: { type: 'prometheus', uid: 'prometheus' },
        expr: |||
          topk(8,
            sum by (groupname) (
              rate(namedprocess_namegroup_major_page_faults_total{groupname!=""}[1m])
            )
          )
        |||,
        legendFormat: '{{groupname}}',
        refId: 'A',
      },
    ],
    title: '💔 MAJOR PAGE FAULTS',
    description: '>0 indicates memory pressure + swapping',
    type: 'timeseries',
  },

  // ✅ FIXED: Thread Count — Tokio/Go apps spawn many threads
  threadExplosion:: {
    datasource: { type: 'prometheus', uid: 'prometheus' },
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
        datasource: { type: 'prometheus', uid: 'prometheus' },
        expr: |||
          topk(10,
            sum by (groupname) (
              namedprocess_namegroup_num_threads{groupname!=""}
            )
          )
        |||,
        legendFormat: '{{groupname}}',
        refId: 'A',
      },
    ],
    title: '🧵 THREAD COUNT',
    description: 'Thread explosion — file-host uses Tokio (async)',
    type: 'timeseries',
  },

  // ✅ FIXED: File Descriptor Usage — worst_fd_ratio is correct
  fileDescriptorUsage:: {
    datasource: { type: 'prometheus', uid: 'prometheus' },
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
        datasource: { type: 'prometheus', uid: 'prometheus' },
        expr: |||
          topk(10,
            max by (groupname) (
              namedprocess_namegroup_worst_fd_ratio{groupname!=""}
            )
          )
        |||,
        legendFormat: '{{groupname}}',
        refId: 'A',
      },
    ],
    title: '📁 FILE DESCRIPTOR USAGE RATIO',
    description: 'FD exhaustion risk — ratio of used/available',
    type: 'timeseries',
  },

  // ========== SYSTEM-WIDE HEALTH INDICATORS ==========

  // ✅ FIXED: Disk I/O Queue — node_disk_io_now is correct
  diskIoQueue:: {
    datasource: { type: 'prometheus', uid: 'prometheus' },
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
          properties: [{ id: 'color', value: { mode: 'fixed', fixedColor: 'orange' } }],
        },
      ],
    },
    options: {
      legend: { displayMode: 'visible', placement: 'bottom' },
      tooltip: { mode: 'multi', sort: 'desc' },
    },
    targets: [
      {
        datasource: { type: 'prometheus', uid: 'prometheus' },
        expr: 'avg_over_time(node_disk_io_now[5m])',
        legendFormat: 'Avg Queue Depth (5m)',
        refId: 'A',
      },
      {
        datasource: { type: 'prometheus', uid: 'prometheus' },
        expr: 'node_disk_io_now',
        legendFormat: 'Current Queue Depth',
        refId: 'B',
      },
    ],
    title: '💿 DISK I/O QUEUE DEPTH',
    description: 'Queue >10 = disk bottleneck',
    type: 'timeseries',
  },

  // ✅ FIXED: Swap Thrashing — combine usage % and page faults
  swapThrashing:: {
    datasource: { type: 'prometheus', uid: 'prometheus' },
    fieldConfig: {
      defaults: { custom: {}, unit: 'percent' },
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
        datasource: { type: 'prometheus', uid: 'prometheus' },
        expr: |||
          (
            (node_memory_SwapTotal_bytes - node_memory_SwapFree_bytes)
            /
            node_memory_SwapTotal_bytes
          ) * 100
        |||,
        legendFormat: 'Swap Usage %',
        refId: 'A',
      },
      {
        datasource: { type: 'prometheus', uid: 'prometheus' },
        expr: 'rate(node_vmstat_pgmajfault[1m])',
        legendFormat: 'Major Page Faults/sec',
        refId: 'B',
      },
    ],
    title: '🔄 SWAP USAGE & THRASHING',
    description: 'Swap >5% or page faults >1/sec = memory pressure',
    type: 'timeseries',
  },

  // ✅ FIXED: Container Process Limits — cAdvisor process count
  containerProcessLimits:: {
    datasource: { type: 'prometheus', uid: 'prometheus' },
    fieldConfig: {
      defaults: {
        custom: { align: 'auto', displayMode: 'auto' },
        unit: 'none',
        thresholds: {
          mode: 'absolute',
          steps: [
            { color: 'green', value: null },
            { color: 'yellow', value: 150 },  // file-host: 200
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
        datasource: { type: 'prometheus', uid: 'prometheus' },
        expr: |||
          container_processes{
            name!="",
            container_label_com_docker_compose_service!=""
          }
        |||,
        format: 'table',
        instant: true,
        refId: 'A',
      },
    ],
    title: '🐳 CONTAINER PROCESS COUNTS',
    description: 'Current vs pids_limit — file-host: 200, metabase: 100',
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
            container_label_com_docker_compose_service: 'Container Name',
            Value: 'Process Count',
          },
        },
      },
    ],
    type: 'table',
  },
}
