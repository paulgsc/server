local grafana = import 'grafonnet/grafana.libsonnet';
local annotation = grafana.annotation;

{
  serviceRestartAnnotation:: annotation.new(
    name='Service Restart',
    datasource='${DS_PROMETHEUS}',
    expr='changes(up[5m]) > 0',
    iconColor='red',
    titleFormat='Restart',
    textFormat='Service restarted',
    tagKeys=['instance'],
    step='10s',
    enable=true,
  ),
}
