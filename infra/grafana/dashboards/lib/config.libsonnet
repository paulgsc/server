// config.libsonnet
{
  // The one provisioned datasource (infra/grafana/provisioning/datasources/datasources.yml
  // pins uid: prometheus). Every panel in every surviving dashboard takes
  // its datasource reference from here — see #218. One datasource does not
  // need a template-variable picker, so this is the uid directly rather
  // than an indirection through `$datasource`.
  prometheusDataSource: {
    type: 'prometheus',
    uid: 'prometheus',
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
