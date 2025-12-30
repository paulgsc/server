
local grafana = import 'grafonnet/grafana.libsonnet';

{
    // Low Cache Hit Rate Alert
    lowCacheHitRate(threshold=70, duration='5m'):: {
uid: 'cache_hit_rate_low',
         title: 'Low Cache Hit Rate',
         condition: 'B',
         data: [
         {
refId: 'A',
       queryType: '',
       relativeTimeRange: {
from: 300,
      to: 0,
       },
model: {
expr: |||
          (
           sum(rate(cache_operations_total{result="hit"}[5m])) /
           sum(
               rate(cache_operations_total{result="hit"}[5m]) +
               rate(cache_operations_total{result="miss"}[5m])
              )
          ) * 100
          |||,
      refId: 'A',
       },
         },
         {
refId: 'B',
       queryType: '',
       relativeTimeRange: {
from: 0,
      to: 0,
       },
model: {
expr: '$A < ' + threshold,
      refId: 'B',
       },
         },
         ],
         noDataState: 'NoData',
         execErrState: 'Alerting',
         'for': duration,
         annotations: {
description: 'Cache hit rate has fallen below ' + threshold + '% for more than ' + duration,
             summary: 'Cache performance degraded',
         },
labels: {
severity: 'critical',
        },
    },

        // Service Down Alert
        serviceDown(duration='1m'):: {
uid: 'service_down_alert',
     title: 'Service Down',
     condition: 'B',
     data: [
     {
refId: 'A',
       queryType: '',
       relativeTimeRange: {
from: 60,
      to: 0,
       },
model: {
expr: 'up',
      refId: 'A',
       },
     },
     {
refId: 'B',
       queryType: '',
       relativeTimeRange: {
from: 0,
      to: 0,
       },
model: {
expr: '$A == 0',
      refId: 'B',
       },
     },
     ],
     noDataState: 'Alerting',
     execErrState: 'Alerting',
     'for': duration,
     annotations: {
description: 'Service {{$labels.instance}} is down',
             summary: 'Service unavailable',
     },
labels: {
severity: 'critical',
        },
        },

        // High Cache Error Rate Alert
        highCacheErrorRate(threshold=5, duration='3m'):: {
uid: 'high_cache_error_rate',
     title: 'High Cache Error Rate',
     condition: 'B',
     data: [
     {
refId: 'A',
       queryType: '',
       relativeTimeRange: {
from: 300,
      to: 0,
       },
model: {
expr: |||
          (
           sum(rate(cache_operations_total{result="error"}[5m])) /
           sum(rate(cache_operations_total[5m]))
          ) * 100
          |||,
      refId: 'A',
       },
     },
     {
refId: 'B',
       queryType: '',
       relativeTimeRange: {
from: 0,
      to: 0,
       },
model: {
expr: '$A > ' + threshold,
      refId: 'B',
       },
     },
     ],
     noDataState: 'NoData',
     execErrState: 'Alerting',
     'for': duration,
     annotations: {
description: 'Cache error rate is above ' + threshold + '% for {{$labels.handler}}',
             summary: 'High cache error rate detected',
     },
labels: {
severity: 'warning',
        },
        },

        // High Request Latency Alert
        highLatency(threshold=0.5, duration='2m'):: {
uid: 'high_latency_alert',
     title: 'High Request Latency',
     condition: 'B',
     data: [
     {
refId: 'A',
       queryType: '',
       relativeTimeRange: {
from: 300,
      to: 0,
       },
model: {
expr: 'histogram_quantile(0.95, rate(operation_duration_seconds_bucket[5m]))',
      refId: 'A',
       },
     },
     {
refId: 'B',
       queryType: '',
       relativeTimeRange: {
from: 0,
      to: 0,
       },
model: {
expr: '$A > ' + threshold,
      refId: 'B',
       },
     },
     ],
     noDataState: 'NoData',
     execErrState: 'Alerting',
     'for': duration,
     annotations: {
description: '95th percentile latency is above ' + (threshold * 1000) + 'ms for {{$labels.handler}}',
             summary: 'High latency detected',
     },
labels: {
severity: 'warning',
        },
        },

        // High Request Rate Alert
        highRequestRate(threshold=1000, duration='5m'):: {
uid: 'high_request_rate_alert',
     title: 'High Request Rate',
     condition: 'B',
     data: [
     {
refId: 'A',
       queryType: '',
       relativeTimeRange: {
from: 300,
      to: 0,
       },
model: {
expr: 'sum(rate(operation_duration_seconds_count[5m]))',
      refId: 'A',
       },
     },
     {
refId: 'B',
       queryType: '',
       relativeTimeRange: {
from: 0,
      to: 0,
       },
model: {
expr: '$A > ' + threshold,
      refId: 'B',
       },
     },
     ],
     noDataState: 'NoData',
     execErrState: 'Alerting',
     'for': duration,
     annotations: {
description: 'Request rate is above ' + threshold + ' req/s for more than ' + duration,
             summary: 'High request rate detected',
     },
labels: {
severity: 'info',
        },
        },

        // Memory Usage Alert
        highMemoryUsage(threshold=85, duration='5m'):: {
uid: 'high_memory_usage_alert',
     title: 'High Memory Usage',
     condition: 'B',
     data: [
     {
refId: 'A',
       queryType: '',
       relativeTimeRange: {
from: 300,
      to: 0,
       },
model: {
expr: '(1 - (node_memory_MemAvailable_bytes / node_memory_MemTotal_bytes)) * 100',
      refId: 'A',
       },
     },
     {
refId: 'B',
       queryType: '',
       relativeTimeRange: {
from: 0,
      to: 0,
       },
model: {
expr: '$A > ' + threshold,
      refId: 'B',
       },
     },
     ],
     noDataState: 'NoData',
     execErrState: 'Alerting',
     'for': duration,
     annotations: {
description: 'Memory usage is above ' + threshold + '% on {{$labels.instance}}',
             summary: 'High memory usage detected',
     },
labels: {
severity: 'warning',
        },
        },

        // Disk Space Alert
        lowDiskSpace(threshold=90, duration='5m'):: {
uid: 'low_disk_space_alert',
     title: 'Low Disk Space',
     condition: 'B',
     data: [
     {
refId: 'A',
       queryType: '',
       relativeTimeRange: {
from: 300,
      to: 0,
       },
model: {
expr: '(1 - (node_filesystem_avail_bytes{mountpoint="/"} / node_filesystem_size_bytes{mountpoint="/"})) * 100',
      refId: 'A',
       },
     },
     {
refId: 'B',
       queryType: '',
       relativeTimeRange: {
from: 0,
      to: 0,
       },
model: {
expr: '$A > ' + threshold,
      refId: 'B',
       },
     },
     ],
     noDataState: 'NoData',
     execErrState: 'Alerting',
     'for': duration,
     annotations: {
description: 'Disk usage is above ' + threshold + '% on {{$labels.instance}}',
             summary: 'Low disk space detected',
     },
labels: {
severity: 'critical',
        },
        },
}
