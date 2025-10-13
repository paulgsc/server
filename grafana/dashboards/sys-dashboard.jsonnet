// dashboard.libsonnet - Node System Metrics Dashboard
local panels = import 'lib/node-panels.libsonnet';
local utils = import 'lib/utils.libsonnet';

local dashboard = {
  annotations: {
    list: [
      {
        builtIn: 1,
        datasource: {
          type: 'datasource',
          uid: '${DS_PROMETHEUS}',  // Fixed: use same DS as panels
        },
        enable: true,
        hide: true,
        iconColor: 'rgba(0, 211, 255, 1)',
        name: 'Annotations & Alerts',
        target: {
          limit: 100,
          matchAny: false,
          tags: [],
          type: 'dashboard',
        },
        type: 'dashboard',
      },
    ],
  },
  editable: true,
  fiscalYearStartMonth: 0,
  graphTooltip: 1,
  id: null,  // Let Grafana assign ID on import
  links: [],
  liveNow: false,
  panels: [
    // === Row 1: Overview Stats ===
    panels.systemUptime { gridPos: utils.gridPos(0, 0, 6, 4) },
    panels.cpuCores { gridPos: utils.gridPos(6, 0, 6, 4) },
    panels.memoryTotal { gridPos: utils.gridPos(12, 0, 6, 4) },
    panels.memoryUtilizationPercent { gridPos: utils.gridPos(18, 0, 6, 4) },

    // === Row 2: CPU and Memory (High-Level) ===
    panels.cpuUsage { gridPos: utils.gridPos(0, 4, 12, 8) },
    panels.memoryUsage { gridPos: utils.gridPos(12, 4, 12, 8) },

    // === Row 3: Detailed CPU and Memory Breakdown ===
    panels.cpuByCore { gridPos: utils.gridPos(0, 12, 12, 8) },
    panels.memoryBreakdown { gridPos: utils.gridPos(12, 12, 12, 8) },

    // === Row 4: Load and Processes ===
    panels.loadAverage { gridPos: utils.gridPos(0, 20, 12, 8) },
    panels.processCount { gridPos: utils.gridPos(12, 20, 6, 8) },
    panels.contextSwitches { gridPos: utils.gridPos(18, 20, 6, 8) },

    // === Row 5: Storage Usage ===
    panels.diskUsage { gridPos: utils.gridPos(0, 28, 12, 8) },
    panels.diskSpaceAvailable { gridPos: utils.gridPos(12, 28, 6, 8) },
    panels.filesystemInodes { gridPos: utils.gridPos(18, 28, 6, 8) },

    // === Row 6: Storage I/O Performance ===
    panels.diskIO { gridPos: utils.gridPos(0, 36, 12, 8) },
    panels.filesystemIOPS { gridPos: utils.gridPos(12, 36, 12, 8) },

    // === Row 7: Disk Utilization and Swap ===
    panels.diskUtilization { gridPos: utils.gridPos(0, 44, 12, 8) },
    panels.swapUsage { gridPos: utils.gridPos(12, 44, 12, 8) },

    // === Row 8: Network I/O and Packets ===
    panels.networkIO { gridPos: utils.gridPos(0, 52, 12, 8) },
    panels.networkPackets { gridPos: utils.gridPos(12, 52, 12, 8) },

    // === Row 9: Network Errors and TCP Connections ===
    panels.networkErrors { gridPos: utils.gridPos(0, 60, 12, 8) },
    panels.tcpConnections { gridPos: utils.gridPos(12, 60, 12, 8) },

    // === Row 10: System Health ===
    panels.fileDescriptors { gridPos: utils.gridPos(0, 68, 6, 8) },
    panels.interrupts { gridPos: utils.gridPos(6, 68, 6, 8) },
    panels.forkRate { gridPos: utils.gridPos(12, 68, 6, 8) },
    panels.entropyAvailable { gridPos: utils.gridPos(18, 68, 6, 8) },

    // === Row 11: System Temperature (Optional, may not be present on all systems) ===
    panels.systemTemperature { gridPos: utils.gridPos(0, 76, 24, 8) },
  ],
  refresh: '30s',
  schemaVersion: 38,
  tags: ['node_exporter', 'system', 'infrastructure'],
  templating: {
    list: [],
  },
  time: {
    from: 'now-1h',
    to: 'now',
  },
  timepicker: {
    refresh_intervals: ['5s', '10s', '30s', '1m', '5m', '15m', '30m', '1h', '2h', '1d'],
    time_options: ['5m', '15m', '1h', '6h', '12h', '24h', '2d', '7d', '30d'],
  },
  timezone: '',
  title: 'Node System Metrics',
  uid: 'node-system-dashboard',
  version: 1,
  weekStart: '',
};

// Output the final dashboard
dashboard
