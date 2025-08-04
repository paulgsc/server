local grafana = import 'grafonnet/grafana.libsonnet';
local template = grafana.template;

{
  handlerTemplate:: template.new(
    name='handler',
    label='Handler',
    datasource='${DS_PROMETHEUS}',
    query='label_values(operation_duration_seconds, handler)',
    includeAll=true,
    multi=true,
    refresh='load',
    sort=1,
  ),

  operationTemplate:: template.new(
    name='operation',
    label='Operation',
    datasource='${DS_PROMETHEUS}',
    query='label_values(operation_duration_seconds, operation)',
    includeAll=true,
    multi=true,
    refresh='load',
    sort=1,
  ),
}
