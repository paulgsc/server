// who-dunnit-dashboard.jsonnet
// System forensic dashboard for identifying performance culprits
// Designed for docker-compose stack with file-host, metabase, grafana, prometheus, redis

local forensicPanels = import 'lib/forensic-panels.libsonnet';

// Utility function for grid positioning
local gridPos(x, y, w, h) = {
  x: x,
  y: y,
  w: w,
  h: h,
};

local dashboard = {
  annotations: {
    list: [
      {
        builtIn: 1,
        datasource: {
          type: 'datasource',
          uid: 'prometheus',
        },
        enable: true,
        hide: true,
        iconColor: 'rgba(0, 211, 255, 1)',
        name: 'Annotations & Alerts',
        type: 'dashboard',
      },
      {
        datasource: {
          type: 'prometheus',
          uid: 'prometheus',
        },
        enable: true,
        expr: 'ALERTS{alertstate="firing"}',
        iconColor: 'red',
        name: 'System Alerts',
        step: '60s',
        tagKeys: 'alertname',
        textFormat: '{{alertname}}: {{instance}}',
        titleFormat: 'Alert: {{alertname}}',
        type: 'dashboard',
      },
    ],
  },
  description: 'Forensic dashboard to identify system hang culprits in docker-compose environment',
  editable: true,
  fiscalYearStartMonth: 0,
  graphTooltip: 1,
  id: null,
  links: [
    {
      asDropdown: false,
      icon: 'external link',
      includeVars: false,
      keepTime: false,
      tags: [],
      targetBlank: true,
      title: 'Process Exporter Docs',
      tooltip: 'Process exporter documentation and metrics reference',
      type: 'link',
      url: 'https://github.com/ncabatoff/process-exporter',
    },
    {
      asDropdown: false,
      icon: 'external link',
      includeVars: false,
      keepTime: false,
      tags: [],
      targetBlank: true,
      title: 'cAdvisor Container Metrics',
      tooltip: 'Container advisor documentation',
      type: 'link',
      url: 'https://github.com/google/cadvisor',
    },
    {
      asDropdown: false,
      icon: 'cloud',
      includeVars: true,
      keepTime: true,
      tags: [],
      targetBlank: false,
      title: 'Container Logs',
      tooltip: 'Jump to container log analysis',
      type: 'link',
      url: '/d/container-logs',
    },
  ],
  liveNow: false,
  panels: [
    // ========== ROW 1: CRITICAL SYSTEM INVARIANTS (RED ALERT ZONE) ==========

    // System hang detector - primary alert
    forensicPanels.systemHangDetector {
      id: 1,
      gridPos: gridPos(0, 0, 8, 6),
    },

    // Request rate invariant violation
    forensicPanels.requestRateInvariant {
      id: 2,
      gridPos: gridPos(8, 0, 8, 6),
    },

    // Blocked processes count
    forensicPanels.blockedProcesses {
      id: 3,
      gridPos: gridPos(16, 0, 8, 6),
    },

    // ========== ROW 2: PRIMARY SUSPECTS - WHO DUNNIT ==========

    // Top CPU offenders table
    forensicPanels.topCpuOffenders {
      id: 4,
      gridPos: gridPos(0, 6, 8, 8),
    },

    // Top memory consumers
    forensicPanels.topMemoryConsumers {
      id: 5,
      gridPos: gridPos(8, 6, 8, 8),
    },

    // Top I/O bandwidth criminals
    forensicPanels.topIoOffenders {
      id: 6,
      gridPos: gridPos(16, 6, 8, 8),
    },

    // ========== ROW 3: SYSTEM CORRELATION MATRIX ==========

    // Load vs CPU divergence - primary hang indicator
    forensicPanels.loadVsCpuDivergence {
      id: 7,
      gridPos: gridPos(0, 14, 12, 6),
    },

    // Process state distribution
    forensicPanels.processStates {
      id: 8,
      gridPos: gridPos(12, 14, 12, 6),
    },

    // ========== ROW 4: CONTAINER EVIDENCE ==========

    // Container CPU usage with service highlighting
    forensicPanels.containerCpuUsage {
      id: 9,
      gridPos: gridPos(0, 20, 8, 6),
    },

    // Container memory pressure vs limits
    forensicPanels.containerMemoryPressure {
      id: 10,
      gridPos: gridPos(8, 20, 8, 6),
    },

    // Container process count vs limits
    forensicPanels.containerProcessLimits {
      id: 11,
      gridPos: gridPos(16, 20, 8, 6),
    },

    // ========== ROW 5: FORENSIC EVIDENCE - RESOURCE BEHAVIOR ==========

    // Context switches storm detection
    forensicPanels.contextSwitches {
      id: 12,
      gridPos: gridPos(0, 26, 8, 6),
    },

    // Major page faults (swap thrashing)
    forensicPanels.majorPageFaults {
      id: 13,
      gridPos: gridPos(8, 26, 8, 6),
    },

    // Thread explosion detection
    forensicPanels.threadExplosion {
      id: 14,
      gridPos: gridPos(16, 26, 8, 6),
    },

    // ========== ROW 6: RESOURCE LEAKS AND EXHAUSTION ==========

    // File descriptor usage ratios
    forensicPanels.fileDescriptorUsage {
      id: 15,
      gridPos: gridPos(0, 32, 12, 6),
    },

    // Container I/O bandwidth
    forensicPanels.containerIoBandwidth {
      id: 16,
      gridPos: gridPos(12, 32, 12, 6),
    },

    // ========== ROW 7: SYSTEM-WIDE HEALTH INDICATORS ==========

    // Disk I/O queue depth and bottlenecks
    forensicPanels.diskIoQueue {
      id: 17,
      gridPos: gridPos(0, 38, 12, 6),
    },

    // Swap usage and thrashing indicators
    forensicPanels.swapThrashing {
      id: 18,
      gridPos: gridPos(12, 38, 12, 6),
    },
  ],
  refresh: '5s',
  schemaVersion: 38,
  style: 'dark',
  tags: [
    'forensics',
    'process-monitoring',
    'system-hangs',
    'who-dunnit',
    'docker-compose',
    'file-host',
    'performance',
  ],
  templating: {
    list: [
      {
        current: {
          selected: false,
          text: 'prometheus',
          value: 'prometheus',
        },
        hide: 0,
        includeAll: false,
        label: 'Data Source',
        multi: false,
        name: 'datasource',
        options: [],
        query: 'prometheus',
        queryValue: '',
        refresh: 1,
        regex: '',
        skipUrlSync: false,
        type: 'datasource',
      },
      {
        current: {
          selected: true,
          text: ['All'],
          value: ['$__all'],
        },
        datasource: {
          type: 'prometheus',
          uid: '${datasource}',
        },
        definition: 'label_values(namedprocess_namegroup_cpu_seconds_total, groupname)',
        hide: 0,
        includeAll: true,
        label: 'Process Group',
        multi: true,
        name: 'process_group',
        options: [],
        query: {
          query: 'label_values(namedprocess_namegroup_cpu_seconds_total, groupname)',
          refId: 'StandardVariableQuery',
        },
        refresh: 1,
        regex: '',
        skipUrlSync: false,
        sort: 1,
        type: 'query',
      },
      {
        current: {
          selected: true,
          text: ['All'],
          value: ['$__all'],
        },
        datasource: {
          type: 'prometheus',
          uid: '${datasource}',
        },
        definition: 'label_values(container_cpu_usage_seconds_total{name!=""}, name)',
        hide: 0,
        includeAll: true,
        label: 'Container',
        multi: true,
        name: 'container',
        options: [],
        query: {
          query: 'label_values(container_cpu_usage_seconds_total{name!=""}, name)',
          refId: 'StandardVariableQuery',
        },
        refresh: 1,
        regex: '',
        skipUrlSync: false,
        sort: 1,
        type: 'query',
      },
    ],
  },
  time: {
    from: 'now-30m',
    to: 'now',
  },
  timepicker: {
    refresh_intervals: ['5s', '10s', '30s', '1m', '5m', '15m', '30m', '1h'],
    time_options: ['5m', '15m', '1h', '6h', '12h', '24h', '2d', '7d', '30d'],
  },
  timezone: '',
  title: '🕵️ WHO DUNNIT - System Forensic Dashboard',
  uid: 'who-dunnit-forensic',
  version: 1,
  weekStart: '',
};

dashboard
