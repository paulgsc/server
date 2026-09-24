// config.libsonnet
{
  // The provisioned metrics datasource (infra/grafana/provisioning/datasources/datasources.yml
  // pins uid: prometheus). Every metric panel in every surviving dashboard
  // takes its datasource reference from here — see #218. No
  // template-variable picker needed for a single metrics datasource, so
  // this is the uid directly rather than an indirection through
  // `$datasource`.
  prometheusDataSource: {
    type: 'prometheus',
    uid: 'prometheus',
  },

  // The provisioned logs datasource, same pinned-uid reasoning as
  // prometheusDataSource above — see datasources.yml's own comment on it.
  lokiDataSource: {
    type: 'loki',
    uid: 'loki',
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
