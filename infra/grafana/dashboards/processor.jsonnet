// who-dunnit-dashboard.jsonnet
// System forensic dashboard for identifying performance culprits
// Designed for docker-compose stack with file-host, metabase, grafana, prometheus, redis

local forensicPanels = import 'lib/forensic-panels.libsonnet';
local panelDefaults = import 'lib/panel-defaults.libsonnet';
local sympathy = import 'lib/sympathy-panels.libsonnet';

// Utility function for grid positioning
local gridPos(x, y, w, h) = {
  x: x,
  y: y,
  w: w,
  h: h,
};

local row(title, y, id) = { type: 'row', title: title, id: id, collapsed: false, gridPos: gridPos(0, y, 24, 1), panels: [] };

// A collapsed row carries its panels inside itself; `hardenAll` is applied to
// them here because the dashboard-level one only walks the top-level list.
local collapsedRow(title, y, id, children) = row(title, y, id) { collapsed: true, panels: panelDefaults.hardenAll(children) };

local dashboard = {
  annotations: {
    list: [
      {
        builtIn: 1,
        // Grafana's own built-in "Annotations & Alerts" store, not
        // Prometheus — always present under this literal uid. See #218.
        datasource: { type: 'grafana', uid: '-- Grafana --' },
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
  description: 'Mechanical sympathy for the host and the services on it: which resource is busy or saturated, and who is responsible. The original hang forensics are the collapsed row at the bottom.',
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
  // Mechanical sympathy at a glance: eight verdict tiles (is any resource
  // busy, is anything waiting on it, is anything about to break), then one
  // band per resource saying who is responsible — see
  // sympathy-panels.libsonnet's header for the USE-method shape. The
  // original forensic panels, for "the host hung, what was it doing", are
  // the collapsed row at the bottom.
  panels: panelDefaults.hardenAll([
    sympathy.cpuBusy { id: 20, gridPos: gridPos(0, 0, 3, 4) },
    sympathy.cpuPressure { id: 21, gridPos: gridPos(3, 0, 3, 4) },
    sympathy.memUsed { id: 22, gridPos: gridPos(6, 0, 3, 4) },
    sympathy.memPressure { id: 23, gridPos: gridPos(9, 0, 3, 4) },
    sympathy.diskBusy { id: 24, gridPos: gridPos(12, 0, 3, 4) },
    sympathy.ioPressure { id: 25, gridPos: gridPos(15, 0, 3, 4) },
    sympathy.diskFree { id: 26, gridPos: gridPos(18, 0, 3, 4) },
    sympathy.oomKills { id: 27, gridPos: gridPos(21, 0, 3, 4) },

    sympathy.collectors { id: 19, gridPos: gridPos(0, 4, 12, 3) },
    sympathy.cpuAccounted { id: 28, gridPos: gridPos(12, 4, 6, 3) },
    sympathy.cores { id: 29, gridPos: gridPos(18, 4, 6, 3) },

    row('CPU — who is using it, and who is waiting', 7, 40),
    sympathy.cpuByProcess { id: 30, gridPos: gridPos(0, 8, 8, 9) },
    sympathy.cpuByContainer { id: 31, gridPos: gridPos(8, 8, 8, 9) },
    sympathy.cpuTrend { id: 32, gridPos: gridPos(16, 8, 8, 9) },
    sympathy.cpuThrottling { id: 33, gridPos: gridPos(0, 17, 12, 7) },
    forensicPanels.loadVsCpuDivergence { id: 7, gridPos: gridPos(12, 17, 12, 7) },

    row('Memory — who is holding it', 24, 41),
    sympathy.memByProcess { id: 34, gridPos: gridPos(0, 25, 8, 9) },
    sympathy.memByContainer { id: 35, gridPos: gridPos(8, 25, 8, 9) },
    sympathy.memTrend { id: 36, gridPos: gridPos(16, 25, 8, 9) },

    row('Disk — who is filling and hitting it', 34, 42),
    sympathy.diskSpace { id: 37, gridPos: gridPos(0, 35, 8, 9) },
    sympathy.ioByProcess { id: 38, gridPos: gridPos(8, 35, 8, 9) },
    sympathy.diskTrend { id: 39, gridPos: gridPos(16, 35, 8, 9) },

    collapsedRow('Forensics — the host hung, what was it doing?', 44, 43, [
      forensicPanels.systemHangDetector { id: 1, gridPos: gridPos(0, 45, 12, 5) },
      forensicPanels.blockedProcesses { id: 3, gridPos: gridPos(12, 45, 12, 5) },
      forensicPanels.topCpuOffenders { id: 4, gridPos: gridPos(0, 50, 8, 8) },
      forensicPanels.topMemoryConsumers { id: 5, gridPos: gridPos(8, 50, 8, 8) },
      forensicPanels.topIoOffenders { id: 6, gridPos: gridPos(16, 50, 8, 8) },
      forensicPanels.processStates { id: 8, gridPos: gridPos(0, 58, 12, 6) },
      forensicPanels.containerProcessLimits { id: 11, gridPos: gridPos(12, 58, 12, 6) },
      forensicPanels.containerCpuUsage { id: 9, gridPos: gridPos(0, 64, 8, 6) },
      forensicPanels.containerMemoryPressure { id: 10, gridPos: gridPos(8, 64, 8, 6) },
      forensicPanels.containerIoBandwidth { id: 16, gridPos: gridPos(16, 64, 8, 6) },
      forensicPanels.contextSwitches { id: 12, gridPos: gridPos(0, 70, 8, 6) },
      forensicPanels.majorPageFaults { id: 13, gridPos: gridPos(8, 70, 8, 6) },
      forensicPanels.threadExplosion { id: 14, gridPos: gridPos(16, 70, 8, 6) },
      forensicPanels.fileDescriptorUsage { id: 15, gridPos: gridPos(0, 76, 8, 6) },
      forensicPanels.diskIoQueue { id: 17, gridPos: gridPos(8, 76, 8, 6) },
      forensicPanels.swapThrashing { id: 18, gridPos: gridPos(16, 76, 8, 6) },
    ]),
  ]),
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
          selected: true,
          text: ['All'],
          value: ['$__all'],
        },
        // One datasource does not need a picker; #218 drops the
        // `datasource`-type template variable this used to route through
        // and references the provisioned uid directly.
        datasource: {
          type: 'prometheus',
          uid: 'prometheus',
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
          uid: 'prometheus',
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
  title: '🕵️ WHO DUNNIT — mechanical sympathy',
  uid: 'who-dunnit-forensic',
  version: 1,
  weekStart: '',
};

dashboard
