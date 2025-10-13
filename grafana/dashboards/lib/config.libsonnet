// config.libsonnet
{
  // Prometheus datasource
  prometheusDataSource: {
    type: 'prometheus',
    uid: 'PB0E20699',
  },

  // Blackbox job names (make them configurable!)
  blackbox: {
    tcpJob: 'websocket_blackbox_tcp',
    httpJob: 'websocket_blackbox_http',
  },

  // Optional: SLA thresholds (for reuse in alerts or panels)
  sla: {
    targetUptimePercent: 99.9,
    criticalUptimePercent: 99.0,
  },

}
