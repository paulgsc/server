// config.libsonnet
{
  // Prometheus datasource
  prometheusDataSource: {
    type: 'prometheus',
    uid: '$datasource',  // Grafana template var
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
