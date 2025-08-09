// node_panels.libsonnet - Panel definitions for node_exporter metrics
{
  // CPU Usage
  cpuUsage: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'palette-classic',
        },
        custom: {
          axisCenteredZero: false,
          axisColorMode: 'text',
          axisLabel: '',
          axisPlacement: 'auto',
          barAlignment: 0,
          drawStyle: 'line',
          fillOpacity: 10,
          gradientMode: 'none',
          hideFrom: {
            legend: false,
            tooltip: false,
            vis: false,
          },
          lineInterpolation: 'linear',
          lineWidth: 1,
          pointSize: 5,
          scaleDistribution: {
            type: 'linear',
          },
          showPoints: 'never',
          spanNulls: false,
          stacking: {
            group: 'A',
            mode: 'none',
          },
          thresholdsStyle: {
            mode: 'off',
          },
        },
        mappings: [],
        max: 100,
        min: 0,
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
            {
              color: 'red',
              value: 80,
            },
          ],
        },
        unit: 'percent',
      },
      overrides: [],
    },
    options: {
      legend: {
        calcs: [],
        displayMode: 'list',
        placement: 'bottom',
        showLegend: true,
      },
      tooltip: {
        mode: 'single',
        sort: 'none',
      },
    },
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: '100 - (avg(irate(node_cpu_seconds_total{mode="idle"}[5m])) * 100)',
        legendFormat: 'CPU Usage',
        range: true,
        refId: 'A',
      },
    ],
    title: 'CPU Usage',
    type: 'timeseries',
  },

  // Memory Usage
  memoryUsage: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'palette-classic',
        },
        custom: {
          axisCenteredZero: false,
          axisColorMode: 'text',
          axisLabel: '',
          axisPlacement: 'auto',
          barAlignment: 0,
          drawStyle: 'line',
          fillOpacity: 10,
          gradientMode: 'none',
          hideFrom: {
            legend: false,
            tooltip: false,
            vis: false,
          },
          lineInterpolation: 'linear',
          lineWidth: 1,
          pointSize: 5,
          scaleDistribution: {
            type: 'linear',
          },
          showPoints: 'never',
          spanNulls: false,
          stacking: {
            group: 'A',
            mode: 'normal',
          },
          thresholdsStyle: {
            mode: 'off',
          },
        },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
            {
              color: 'red',
              value: 80,
            },
          ],
        },
        unit: 'bytes',
      },
      overrides: [],
    },
    options: {
      legend: {
        calcs: [],
        displayMode: 'list',
        placement: 'bottom',
        showLegend: true,
      },
      tooltip: {
        mode: 'single',
        sort: 'none',
      },
    },
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'node_memory_MemTotal_bytes - node_memory_MemAvailable_bytes',
        legendFormat: 'Used Memory',
        range: true,
        refId: 'A',
      },
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'node_memory_MemAvailable_bytes',
        legendFormat: 'Available Memory',
        range: true,
        refId: 'B',
      },
    ],
    title: 'Memory Usage',
    type: 'timeseries',
  },

  // Memory Breakdown (Detailed Components)
  memoryBreakdown: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'palette-classic',
        },
        custom: {
          axisCenteredZero: false,
          axisColorMode: 'text',
          axisLabel: '',
          axisPlacement: 'auto',
          barAlignment: 0,
          drawStyle: 'line',
          fillOpacity: 10,
          gradientMode: 'none',
          hideFrom: {
            legend: false,
            tooltip: false,
            vis: false,
          },
          lineInterpolation: 'linear',
          lineWidth: 1,
          pointSize: 5,
          scaleDistribution: {
            type: 'linear',
          },
          showPoints: 'never',
          spanNulls: false,
          stacking: {
            group: 'A',
            mode: 'normal',
          },
          thresholdsStyle: {
            mode: 'off',
          },
        },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
            {
              color: 'red',
              value: 80,
            },
          ],
        },
        unit: 'bytes',
      },
      overrides: [],
    },
    options: {
      legend: {
        calcs: [],
        displayMode: 'list',
        placement: 'bottom',
        showLegend: true,
      },
      tooltip: {
        mode: 'single',
        sort: 'none',
      },
    },
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'node_memory_MemTotal_bytes - node_memory_MemFree_bytes - node_memory_Buffers_bytes - node_memory_Cached_bytes',
        legendFormat: 'Used',
        range: true,
        refId: 'A',
      },
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'node_memory_Buffers_bytes',
        legendFormat: 'Buffers',
        range: true,
        refId: 'B',
      },
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'node_memory_Cached_bytes',
        legendFormat: 'Cached',
        range: true,
        refId: 'C',
      },
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'node_memory_MemFree_bytes',
        legendFormat: 'Free',
        range: true,
        refId: 'D',
      },
    ],
    title: 'Memory Breakdown',
    type: 'timeseries',
  },

  // Network Packets
  networkPackets: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'palette-classic',
        },
        custom: {
          axisCenteredZero: false,
          axisColorMode: 'text',
          axisLabel: '',
          axisPlacement: 'auto',
          barAlignment: 0,
          drawStyle: 'line',
          fillOpacity: 10,
          gradientMode: 'none',
          hideFrom: {
            legend: false,
            tooltip: false,
            vis: false,
          },
          lineInterpolation: 'linear',
          lineWidth: 1,
          pointSize: 5,
          scaleDistribution: {
            type: 'linear',
          },
          showPoints: 'never',
          spanNulls: false,
          stacking: {
            group: 'A',
            mode: 'none',
          },
          thresholdsStyle: {
            mode: 'off',
          },
        },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
            {
              color: 'red',
              value: 80,
            },
          ],
        },
        unit: 'pps',
      },
      overrides: [
        {
          matcher: {
            id: 'byRegexp',
            options: '/.*transmit.*/',
          },
          properties: [
            {
              id: 'custom.transform',
              value: 'negative-Y',
            },
          ],
        },
      ],
    },
    options: {
      legend: {
        calcs: [],
        displayMode: 'list',
        placement: 'bottom',
        showLegend: true,
      },
      tooltip: {
        mode: 'single',
        sort: 'none',
      },
    },
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'irate(node_network_receive_packets_total{device!="lo"}[5m])',
        legendFormat: '{{device}} receive',
        range: true,
        refId: 'A',
      },
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'irate(node_network_transmit_packets_total{device!="lo"}[5m])',
        legendFormat: '{{device}} transmit',
        range: true,
        refId: 'B',
      },
    ],
    title: 'Network Packets/sec',
    type: 'timeseries',
  },

  // Disk Space Available
  diskSpaceAvailable: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'thresholds',
        },
        mappings: [],
        max: 100,
        min: 0,
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'red',
              value: null,
            },
            {
              color: 'yellow',
              value: 20,
            },
            {
              color: 'green',
              value: 40,
            },
          ],
        },
        unit: 'percent',
      },
      overrides: [],
    },
    options: {
      orientation: 'auto',
      reduceOptions: {
        calcs: ['lastNotNull'],
        fields: '',
        values: false,
      },
      showThresholdLabels: false,
      showThresholdMarkers: true,
    },
    pluginVersion: '10.2.0',
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: '100 * (node_filesystem_avail_bytes{fstype!="tmpfs"} / node_filesystem_size_bytes{fstype!="tmpfs"})',
        legendFormat: '{{mountpoint}}',
        range: true,
        refId: 'A',
      },
    ],
    title: 'Disk Space Available',
    type: 'gauge',
  },

  // File Descriptors
  fileDescriptors: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'palette-classic',
        },
        custom: {
          axisCenteredZero: false,
          axisColorMode: 'text',
          axisLabel: '',
          axisPlacement: 'auto',
          barAlignment: 0,
          drawStyle: 'line',
          fillOpacity: 10,
          gradientMode: 'none',
          hideFrom: {
            legend: false,
            tooltip: false,
            vis: false,
          },
          lineInterpolation: 'linear',
          lineWidth: 1,
          pointSize: 5,
          scaleDistribution: {
            type: 'linear',
          },
          showPoints: 'never',
          spanNulls: false,
          stacking: {
            group: 'A',
            mode: 'none',
          },
          thresholdsStyle: {
            mode: 'off',
          },
        },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
            {
              color: 'red',
              value: 80,
            },
          ],
        },
        unit: 'short',
      },
      overrides: [],
    },
    options: {
      legend: {
        calcs: [],
        displayMode: 'list',
        placement: 'bottom',
        showLegend: true,
      },
      tooltip: {
        mode: 'single',
        sort: 'none',
      },
    },
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'node_filefd_allocated',
        legendFormat: 'Allocated',
        range: true,
        refId: 'A',
      },
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'node_filefd_maximum',
        legendFormat: 'Maximum',
        range: true,
        refId: 'B',
      },
    ],
    title: 'File Descriptors',
    type: 'timeseries',
  },

  // Network Errors
  networkErrors: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'palette-classic',
        },
        custom: {
          axisCenteredZero: false,
          axisColorMode: 'text',
          axisLabel: '',
          axisPlacement: 'auto',
          barAlignment: 0,
          drawStyle: 'line',
          fillOpacity: 10,
          gradientMode: 'none',
          hideFrom: {
            legend: false,
            tooltip: false,
            vis: false,
          },
          lineInterpolation: 'linear',
          lineWidth: 1,
          pointSize: 5,
          scaleDistribution: {
            type: 'linear',
          },
          showPoints: 'never',
          spanNulls: false,
          stacking: {
            group: 'A',
            mode: 'none',
          },
          thresholdsStyle: {
            mode: 'off',
          },
        },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
            {
              color: 'red',
              value: 80,
            },
          ],
        },
        unit: 'short',
      },
      overrides: [],
    },
    options: {
      legend: {
        calcs: [],
        displayMode: 'list',
        placement: 'bottom',
        showLegend: true,
      },
      tooltip: {
        mode: 'single',
        sort: 'none',
      },
    },
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'irate(node_network_receive_errs_total{device!="lo"}[5m])',
        legendFormat: '{{device}} RX errors',
        range: true,
        refId: 'A',
      },
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'irate(node_network_transmit_errs_total{device!="lo"}[5m])',
        legendFormat: '{{device}} TX errors',
        range: true,
        refId: 'B',
      },
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'irate(node_network_receive_drop_total{device!="lo"}[5m])',
        legendFormat: '{{device}} RX drops',
        range: true,
        refId: 'C',
      },
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'irate(node_network_transmit_drop_total{device!="lo"}[5m])',
        legendFormat: '{{device}} TX drops',
        range: true,
        refId: 'D',
      },
    ],
    title: 'Network Errors & Drops',
    type: 'timeseries',
  },

  // Disk Utilization
  diskUtilization: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'palette-classic',
        },
        custom: {
          axisCenteredZero: false,
          axisColorMode: 'text',
          axisLabel: '',
          axisPlacement: 'auto',
          barAlignment: 0,
          drawStyle: 'line',
          fillOpacity: 10,
          gradientMode: 'none',
          hideFrom: {
            legend: false,
            tooltip: false,
            vis: false,
          },
          lineInterpolation: 'linear',
          lineWidth: 1,
          pointSize: 5,
          scaleDistribution: {
            type: 'linear',
          },
          showPoints: 'never',
          spanNulls: false,
          stacking: {
            group: 'A',
            mode: 'none',
          },
          thresholdsStyle: {
            mode: 'off',
          },
        },
        mappings: [],
        max: 100,
        min: 0,
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
            {
              color: 'yellow',
              value: 70,
            },
            {
              color: 'red',
              value: 90,
            },
          ],
        },
        unit: 'percent',
      },
      overrides: [],
    },
    options: {
      legend: {
        calcs: [],
        displayMode: 'list',
        placement: 'bottom',
        showLegend: true,
      },
      tooltip: {
        mode: 'single',
        sort: 'none',
      },
    },
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'irate(node_disk_io_time_seconds_total[5m]) * 100',
        legendFormat: '{{device}}',
        range: true,
        refId: 'A',
      },
    ],
    title: 'Disk Utilization %',
    type: 'timeseries',
  },

  // System Temperature
  systemTemperature: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'palette-classic',
        },
        custom: {
          axisCenteredZero: false,
          axisColorMode: 'text',
          axisLabel: '',
          axisPlacement: 'auto',
          barAlignment: 0,
          drawStyle: 'line',
          fillOpacity: 10,
          gradientMode: 'none',
          hideFrom: {
            legend: false,
            tooltip: false,
            vis: false,
          },
          lineInterpolation: 'linear',
          lineWidth: 1,
          pointSize: 5,
          scaleDistribution: {
            type: 'linear',
          },
          showPoints: 'never',
          spanNulls: false,
          stacking: {
            group: 'A',
            mode: 'none',
          },
          thresholdsStyle: {
            mode: 'off',
          },
        },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
            {
              color: 'yellow',
              value: 70,
            },
            {
              color: 'red',
              value: 85,
            },
          ],
        },
        unit: 'celsius',
      },
      overrides: [],
    },
    options: {
      legend: {
        calcs: [],
        displayMode: 'list',
        placement: 'bottom',
        showLegend: true,
      },
      tooltip: {
        mode: 'single',
        sort: 'none',
      },
    },
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'node_hwmon_temp_celsius',
        legendFormat: '{{chip}} {{sensor}}',
        range: true,
        refId: 'A',
      },
    ],
    title: 'System Temperature',
    type: 'timeseries',
  },

  // Interrupts
  interrupts: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'palette-classic',
        },
        custom: {
          axisCenteredZero: false,
          axisColorMode: 'text',
          axisLabel: '',
          axisPlacement: 'auto',
          barAlignment: 0,
          drawStyle: 'line',
          fillOpacity: 10,
          gradientMode: 'none',
          hideFrom: {
            legend: false,
            tooltip: false,
            vis: false,
          },
          lineInterpolation: 'linear',
          lineWidth: 1,
          pointSize: 5,
          scaleDistribution: {
            type: 'linear',
          },
          showPoints: 'never',
          spanNulls: false,
          stacking: {
            group: 'A',
            mode: 'none',
          },
          thresholdsStyle: {
            mode: 'off',
          },
        },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
            {
              color: 'red',
              value: 80,
            },
          ],
        },
        unit: 'ops',
      },
      overrides: [],
    },
    options: {
      legend: {
        calcs: [],
        displayMode: 'list',
        placement: 'bottom',
        showLegend: true,
      },
      tooltip: {
        mode: 'single',
        sort: 'none',
      },
    },
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'irate(node_intr_total[5m])',
        legendFormat: 'Interrupts/sec',
        range: true,
        refId: 'A',
      },
    ],
    title: 'System Interrupts',
    type: 'timeseries',
  },

  // Entropy Available
  entropyAvailable: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'thresholds',
        },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'red',
              value: null,
            },
            {
              color: 'yellow',
              value: 200,
            },
            {
              color: 'green',
              value: 1000,
            },
          ],
        },
        unit: 'short',
      },
      overrides: [],
    },
    maxDataPoints: 100,
    options: {
      colorMode: 'value',
      graphMode: 'area',
      justifyMode: 'auto',
      orientation: 'auto',
      reduceOptions: {
        calcs: ['lastNotNull'],
        fields: '',
        values: false,
      },
      textMode: 'auto',
      wideLayout: true,
    },
    pluginVersion: '10.2.0',
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'node_entropy_available_bits',
        legendFormat: '',
        range: true,
        refId: 'A',
      },
    ],
    title: 'Entropy Available',
    type: 'stat',
  },

  // Swap Usage
  swapUsage: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'palette-classic',
        },
        custom: {
          axisCenteredZero: false,
          axisColorMode: 'text',
          axisLabel: '',
          axisPlacement: 'auto',
          barAlignment: 0,
          drawStyle: 'line',
          fillOpacity: 10,
          gradientMode: 'none',
          hideFrom: {
            legend: false,
            tooltip: false,
            vis: false,
          },
          lineInterpolation: 'linear',
          lineWidth: 1,
          pointSize: 5,
          scaleDistribution: {
            type: 'linear',
          },
          showPoints: 'never',
          spanNulls: false,
          stacking: {
            group: 'A',
            mode: 'normal',
          },
          thresholdsStyle: {
            mode: 'off',
          },
        },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
            {
              color: 'red',
              value: 80,
            },
          ],
        },
        unit: 'bytes',
      },
      overrides: [],
    },
    options: {
      legend: {
        calcs: [],
        displayMode: 'list',
        placement: 'bottom',
        showLegend: true,
      },
      tooltip: {
        mode: 'single',
        sort: 'none',
      },
    },
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'node_memory_SwapTotal_bytes - node_memory_SwapFree_bytes',
        legendFormat: 'Used Swap',
        range: true,
        refId: 'A',
      },
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'node_memory_SwapFree_bytes',
        legendFormat: 'Free Swap',
        range: true,
        refId: 'B',
      },
    ],
    title: 'Swap Usage',
    type: 'timeseries',
  },

  // Filesystem Inodes
  filesystemInodes: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'palette-classic',
        },
        custom: {
          hideFrom: {
            legend: false,
            tooltip: false,
            vis: false,
          },
        },
        mappings: [],
        max: 100,
        min: 0,
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
            {
              color: 'yellow',
              value: 80,
            },
            {
              color: 'red',
              value: 95,
            },
          ],
        },
        unit: 'percent',
      },
      overrides: [],
    },
    options: {
      displayMode: 'lcd',
      minVizHeight: 75,
      minVizWidth: 75,
      orientation: 'horizontal',
      reduceOptions: {
        calcs: ['lastNotNull'],
        fields: '',
        values: false,
      },
      showUnfilled: true,
      sizing: 'auto',
    },
    pluginVersion: '10.2.0',
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: '100 * (1 - (node_filesystem_files_free{fstype!="tmpfs"} / node_filesystem_files{fstype!="tmpfs"}))',
        legendFormat: '{{mountpoint}}',
        range: true,
        refId: 'A',
      },
    ],
    title: 'Filesystem Inodes Usage',
    type: 'bargauge',
  },

  // TCP Connections
  tcpConnections: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'palette-classic',
        },
        custom: {
          axisCenteredZero: false,
          axisColorMode: 'text',
          axisLabel: '',
          axisPlacement: 'auto',
          barAlignment: 0,
          drawStyle: 'line',
          fillOpacity: 10,
          gradientMode: 'none',
          hideFrom: {
            legend: false,
            tooltip: false,
            vis: false,
          },
          lineInterpolation: 'linear',
          lineWidth: 1,
          pointSize: 5,
          scaleDistribution: {
            type: 'linear',
          },
          showPoints: 'never',
          spanNulls: false,
          stacking: {
            group: 'A',
            mode: 'normal',
          },
          thresholdsStyle: {
            mode: 'off',
          },
        },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
            {
              color: 'red',
              value: 80,
            },
          ],
        },
        unit: 'short',
      },
      overrides: [],
    },
    options: {
      legend: {
        calcs: [],
        displayMode: 'list',
        placement: 'bottom',
        showLegend: true,
      },
      tooltip: {
        mode: 'single',
        sort: 'none',
      },
    },
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'node_netstat_Tcp_CurrEstab',
        legendFormat: 'Established',
        range: true,
        refId: 'A',
      },
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'node_sockstat_TCP_tw',
        legendFormat: 'Time Wait',
        range: true,
        refId: 'B',
      },
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'node_sockstat_sockets_used',
        legendFormat: 'Sockets Used',
        range: true,
        refId: 'C',
      },
    ],
    title: 'TCP Connections',
    type: 'timeseries',
  },

  // Fork Rate
  forkRate: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'palette-classic',
        },
        custom: {
          axisCenteredZero: false,
          axisColorMode: 'text',
          axisLabel: '',
          axisPlacement: 'auto',
          barAlignment: 0,
          drawStyle: 'line',
          fillOpacity: 10,
          gradientMode: 'none',
          hideFrom: {
            legend: false,
            tooltip: false,
            vis: false,
          },
          lineInterpolation: 'linear',
          lineWidth: 1,
          pointSize: 5,
          scaleDistribution: {
            type: 'linear',
          },
          showPoints: 'never',
          spanNulls: false,
          stacking: {
            group: 'A',
            mode: 'none',
          },
          thresholdsStyle: {
            mode: 'off',
          },
        },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
            {
              color: 'red',
              value: 80,
            },
          ],
        },
        unit: 'ops',
      },
      overrides: [],
    },
    options: {
      legend: {
        calcs: [],
        displayMode: 'list',
        placement: 'bottom',
        showLegend: true,
      },
      tooltip: {
        mode: 'single',
        sort: 'none',
      },
    },
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'irate(node_forks_total[5m])',
        legendFormat: 'Forks/sec',
        range: true,
        refId: 'A',
      },
    ],
    title: 'Process Fork Rate',
    type: 'timeseries',
  },

  // Memory Utilization Percentage
  memoryUtilizationPercent: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'thresholds',
        },
        mappings: [],
        max: 100,
        min: 0,
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
            {
              color: 'yellow',
              value: 70,
            },
            {
              color: 'red',
              value: 90,
            },
          ],
        },
        unit: 'percent',
      },
      overrides: [],
    },
    options: {
      orientation: 'auto',
      reduceOptions: {
        calcs: ['lastNotNull'],
        fields: '',
        values: false,
      },
      showThresholdLabels: false,
      showThresholdMarkers: true,
    },
    pluginVersion: '10.2.0',
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: '100 * (1 - (node_memory_MemAvailable_bytes / node_memory_MemTotal_bytes))',
        legendFormat: 'Memory %',
        range: true,
        refId: 'A',
      },
    ],
    title: 'Memory Utilization %',
    type: 'gauge',
  },

  // Disk Usage
  diskUsage: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'palette-classic',
        },
        custom: {
          hideFrom: {
            legend: false,
            tooltip: false,
            vis: false,
          },
        },
        mappings: [],
        max: 100,
        min: 0,
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
            {
              color: 'yellow',
              value: 70,
            },
            {
              color: 'red',
              value: 85,
            },
          ],
        },
        unit: 'percent',
      },
      overrides: [],
    },
    options: {
      displayMode: 'lcd',
      minVizHeight: 75,
      minVizWidth: 75,
      orientation: 'horizontal',
      reduceOptions: {
        calcs: ['lastNotNull'],
        fields: '',
        values: false,
      },
      showUnfilled: true,
      sizing: 'auto',
    },
    pluginVersion: '10.2.0',
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: '100 * (1 - (node_filesystem_avail_bytes{fstype!="tmpfs"} / node_filesystem_size_bytes{fstype!="tmpfs"}))',
        legendFormat: '{{mountpoint}}',
        range: true,
        refId: 'A',
      },
    ],
    title: 'Disk Usage by Mount',
    type: 'bargauge',
  },

  // Network I/O
  networkIO: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'palette-classic',
        },
        custom: {
          axisCenteredZero: false,
          axisColorMode: 'text',
          axisLabel: '',
          axisPlacement: 'auto',
          barAlignment: 0,
          drawStyle: 'line',
          fillOpacity: 10,
          gradientMode: 'none',
          hideFrom: {
            legend: false,
            tooltip: false,
            vis: false,
          },
          lineInterpolation: 'linear',
          lineWidth: 1,
          pointSize: 5,
          scaleDistribution: {
            type: 'linear',
          },
          showPoints: 'never',
          spanNulls: false,
          stacking: {
            group: 'A',
            mode: 'none',
          },
          thresholdsStyle: {
            mode: 'off',
          },
        },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
            {
              color: 'red',
              value: 80,
            },
          ],
        },
        unit: 'Bps',
      },
      overrides: [
        {
          matcher: {
            id: 'byRegexp',
            options: '/.*transmit.*/',
          },
          properties: [
            {
              id: 'custom.transform',
              value: 'negative-Y',
            },
          ],
        },
      ],
    },
    options: {
      legend: {
        calcs: [],
        displayMode: 'list',
        placement: 'bottom',
        showLegend: true,
      },
      tooltip: {
        mode: 'single',
        sort: 'none',
      },
    },
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'irate(node_network_receive_bytes_total{device!="lo"}[5m])',
        legendFormat: '{{device}} receive',
        range: true,
        refId: 'A',
      },
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'irate(node_network_transmit_bytes_total{device!="lo"}[5m])',
        legendFormat: '{{device}} transmit',
        range: true,
        refId: 'B',
      },
    ],
    title: 'Network I/O',
    type: 'timeseries',
  },

  // Disk I/O
  diskIO: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'palette-classic',
        },
        custom: {
          axisCenteredZero: false,
          axisColorMode: 'text',
          axisLabel: '',
          axisPlacement: 'auto',
          barAlignment: 0,
          drawStyle: 'line',
          fillOpacity: 10,
          gradientMode: 'none',
          hideFrom: {
            legend: false,
            tooltip: false,
            vis: false,
          },
          lineInterpolation: 'linear',
          lineWidth: 1,
          pointSize: 5,
          scaleDistribution: {
            type: 'linear',
          },
          showPoints: 'never',
          spanNulls: false,
          stacking: {
            group: 'A',
            mode: 'none',
          },
          thresholdsStyle: {
            mode: 'off',
          },
        },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
            {
              color: 'red',
              value: 80,
            },
          ],
        },
        unit: 'Bps',
      },
      overrides: [
        {
          matcher: {
            id: 'byRegexp',
            options: '/.*write.*/',
          },
          properties: [
            {
              id: 'custom.transform',
              value: 'negative-Y',
            },
          ],
        },
      ],
    },
    options: {
      legend: {
        calcs: [],
        displayMode: 'list',
        placement: 'bottom',
        showLegend: true,
      },
      tooltip: {
        mode: 'single',
        sort: 'none',
      },
    },
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'irate(node_disk_read_bytes_total[5m])',
        legendFormat: '{{device}} read',
        range: true,
        refId: 'A',
      },
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'irate(node_disk_written_bytes_total[5m])',
        legendFormat: '{{device}} write',
        range: true,
        refId: 'B',
      },
    ],
    title: 'Disk I/O',
    type: 'timeseries',
  },

  // Load Average
  loadAverage: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'palette-classic',
        },
        custom: {
          axisCenteredZero: false,
          axisColorMode: 'text',
          axisLabel: '',
          axisPlacement: 'auto',
          barAlignment: 0,
          drawStyle: 'line',
          fillOpacity: 10,
          gradientMode: 'none',
          hideFrom: {
            legend: false,
            tooltip: false,
            vis: false,
          },
          lineInterpolation: 'linear',
          lineWidth: 1,
          pointSize: 5,
          scaleDistribution: {
            type: 'linear',
          },
          showPoints: 'never',
          spanNulls: false,
          stacking: {
            group: 'A',
            mode: 'none',
          },
          thresholdsStyle: {
            mode: 'off',
          },
        },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
            {
              color: 'red',
              value: 80,
            },
          ],
        },
        unit: 'short',
      },
      overrides: [],
    },
    options: {
      legend: {
        calcs: [],
        displayMode: 'list',
        placement: 'bottom',
        showLegend: true,
      },
      tooltip: {
        mode: 'single',
        sort: 'none',
      },
    },
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'node_load1',
        legendFormat: '1m',
        range: true,
        refId: 'A',
      },
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'node_load5',
        legendFormat: '5m',
        range: true,
        refId: 'B',
      },
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'node_load15',
        legendFormat: '15m',
        range: true,
        refId: 'C',
      },
    ],
    title: 'Load Average',
    type: 'timeseries',
  },

  // System Uptime
  systemUptime: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'thresholds',
        },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
          ],
        },
        unit: 's',
      },
      overrides: [],
    },
    maxDataPoints: 100,
    options: {
      colorMode: 'value',
      graphMode: 'none',
      justifyMode: 'auto',
      orientation: 'auto',
      reduceOptions: {
        calcs: ['lastNotNull'],
        fields: '',
        values: false,
      },
      textMode: 'auto',
      wideLayout: true,
    },
    pluginVersion: '10.2.0',
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'time() - node_boot_time_seconds',
        legendFormat: '',
        range: true,
        refId: 'A',
      },
    ],
    title: 'System Uptime',
    type: 'stat',
  },

  // CPU Cores
  cpuCores: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'thresholds',
        },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
          ],
        },
        unit: 'short',
      },
      overrides: [],
    },
    maxDataPoints: 100,
    options: {
      colorMode: 'value',
      graphMode: 'none',
      justifyMode: 'auto',
      orientation: 'auto',
      reduceOptions: {
        calcs: ['lastNotNull'],
        fields: '',
        values: false,
      },
      textMode: 'auto',
      wideLayout: true,
    },
    pluginVersion: '10.2.0',
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'count(count by (cpu)(node_cpu_seconds_total))',
        legendFormat: '',
        range: true,
        refId: 'A',
      },
    ],
    title: 'CPU Cores',
    type: 'stat',
  },

  // Memory Total
  memoryTotal: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'thresholds',
        },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
          ],
        },
        unit: 'bytes',
      },
      overrides: [],
    },
    maxDataPoints: 100,
    options: {
      colorMode: 'value',
      graphMode: 'none',
      justifyMode: 'auto',
      orientation: 'auto',
      reduceOptions: {
        calcs: ['lastNotNull'],
        fields: '',
        values: false,
      },
      textMode: 'auto',
      wideLayout: true,
    },
    pluginVersion: '10.2.0',
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'node_memory_MemTotal_bytes',
        legendFormat: '',
        range: true,
        refId: 'A',
      },
    ],
    title: 'Total Memory',
    type: 'stat',
  },

  // CPU by Core
  cpuByCore: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'palette-classic',
        },
        custom: {
          axisCenteredZero: false,
          axisColorMode: 'text',
          axisLabel: '',
          axisPlacement: 'auto',
          barAlignment: 0,
          drawStyle: 'line',
          fillOpacity: 10,
          gradientMode: 'none',
          hideFrom: {
            legend: false,
            tooltip: false,
            vis: false,
          },
          lineInterpolation: 'linear',
          lineWidth: 1,
          pointSize: 5,
          scaleDistribution: {
            type: 'linear',
          },
          showPoints: 'never',
          spanNulls: false,
          stacking: {
            group: 'A',
            mode: 'none',
          },
          thresholdsStyle: {
            mode: 'off',
          },
        },
        mappings: [],
        max: 100,
        min: 0,
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
            {
              color: 'red',
              value: 80,
            },
          ],
        },
        unit: 'percent',
      },
      overrides: [],
    },
    options: {
      legend: {
        calcs: [],
        displayMode: 'list',
        placement: 'bottom',
        showLegend: true,
      },
      tooltip: {
        mode: 'single',
        sort: 'none',
      },
    },
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: '100 - (avg by (cpu)(irate(node_cpu_seconds_total{mode="idle"}[5m])) * 100)',
        legendFormat: 'CPU {{cpu}}',
        range: true,
        refId: 'A',
      },
    ],
    title: 'CPU Usage by Core',
    type: 'timeseries',
  },

  // Filesystem IOPS
  filesystemIOPS: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'palette-classic',
        },
        custom: {
          axisCenteredZero: false,
          axisColorMode: 'text',
          axisLabel: '',
          axisPlacement: 'auto',
          barAlignment: 0,
          drawStyle: 'line',
          fillOpacity: 10,
          gradientMode: 'none',
          hideFrom: {
            legend: false,
            tooltip: false,
            vis: false,
          },
          lineInterpolation: 'linear',
          lineWidth: 1,
          pointSize: 5,
          scaleDistribution: {
            type: 'linear',
          },
          showPoints: 'never',
          spanNulls: false,
          stacking: {
            group: 'A',
            mode: 'none',
          },
          thresholdsStyle: {
            mode: 'off',
          },
        },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
            {
              color: 'red',
              value: 80,
            },
          ],
        },
        unit: 'iops',
      },
      overrides: [
        {
          matcher: {
            id: 'byRegexp',
            options: '/.*write.*/',
          },
          properties: [
            {
              id: 'custom.transform',
              value: 'negative-Y',
            },
          ],
        },
      ],
    },
    options: {
      legend: {
        calcs: [],
        displayMode: 'list',
        placement: 'bottom',
        showLegend: true,
      },
      tooltip: {
        mode: 'single',
        sort: 'none',
      },
    },
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'irate(node_disk_reads_completed_total[5m])',
        legendFormat: '{{device}} reads',
        range: true,
        refId: 'A',
      },
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'irate(node_disk_writes_completed_total[5m])',
        legendFormat: '{{device}} writes',
        range: true,
        refId: 'B',
      },
    ],
    title: 'Disk IOPS',
    type: 'timeseries',
  },

  // Process Count
  processCount: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'palette-classic',
        },
        custom: {
          axisCenteredZero: false,
          axisColorMode: 'text',
          axisLabel: '',
          axisPlacement: 'auto',
          barAlignment: 0,
          drawStyle: 'line',
          fillOpacity: 10,
          gradientMode: 'none',
          hideFrom: {
            legend: false,
            tooltip: false,
            vis: false,
          },
          lineInterpolation: 'linear',
          lineWidth: 1,
          pointSize: 5,
          scaleDistribution: {
            type: 'linear',
          },
          showPoints: 'never',
          spanNulls: false,
          stacking: {
            group: 'A',
            mode: 'none',
          },
          thresholdsStyle: {
            mode: 'off',
          },
        },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
            {
              color: 'red',
              value: 80,
            },
          ],
        },
        unit: 'short',
      },
      overrides: [],
    },
    options: {
      legend: {
        calcs: [],
        displayMode: 'list',
        placement: 'bottom',
        showLegend: true,
      },
      tooltip: {
        mode: 'single',
        sort: 'none',
      },
    },
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'node_procs_running',
        legendFormat: 'Running',
        range: true,
        refId: 'A',
      },
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'node_procs_blocked',
        legendFormat: 'Blocked',
        range: true,
        refId: 'B',
      },
    ],
    title: 'Process Count',
    type: 'timeseries',
  },

  // Context Switches
  contextSwitches: {
    datasource: {
      type: 'prometheus',
      uid: '${DS_PROMETHEUS}',
    },
    fieldConfig: {
      defaults: {
        color: {
          mode: 'palette-classic',
        },
        custom: {
          axisCenteredZero: false,
          axisColorMode: 'text',
          axisLabel: '',
          axisPlacement: 'auto',
          barAlignment: 0,
          drawStyle: 'line',
          fillOpacity: 10,
          gradientMode: 'none',
          hideFrom: {
            legend: false,
            tooltip: false,
            vis: false,
          },
          lineInterpolation: 'linear',
          lineWidth: 1,
          pointSize: 5,
          scaleDistribution: {
            type: 'linear',
          },
          showPoints: 'never',
          spanNulls: false,
          stacking: {
            group: 'A',
            mode: 'none',
          },
          thresholdsStyle: {
            mode: 'off',
          },
        },
        mappings: [],
        thresholds: {
          mode: 'absolute',
          steps: [
            {
              color: 'green',
              value: null,
            },
            {
              color: 'red',
              value: 80,
            },
          ],
        },
        unit: 'ops',
      },
      overrides: [],
    },
    options: {
      legend: {
        calcs: [],
        displayMode: 'list',
        placement: 'bottom',
        showLegend: true,
      },
      tooltip: {
        mode: 'single',
        sort: 'none',
      },
    },
    targets: [
      {
        datasource: {
          type: 'prometheus',
          uid: '${DS_PROMETHEUS}',
        },
        editorMode: 'code',
        expr: 'irate(node_context_switches_total[5m])',
        legendFormat: 'Context Switches/sec',
        range: true,
        refId: 'A',
      },
    ],
    title: 'Context Switches',
    type: 'timeseries',
  },
}
