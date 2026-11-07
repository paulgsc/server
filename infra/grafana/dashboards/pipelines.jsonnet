// pipelines.jsonnet
//
// PIPELINES: each chained process home runs, as a graph whose red node is
// where to dig (server#692). The graphs are lib/pipelines.json, drawn by
// lib/pipeline-panels.libsonnet; scripts/check_pipeline_graph.py holds them
// to paulgsc/some-ui's assurance ledger.
local defs = import 'lib/pipelines.json';
local panels = import 'lib/pipeline-panels.libsonnet';
local panelDefaults = import 'lib/panel-defaults.libsonnet';

local height = 16;

{
  annotations: { list: [] },
  description: 'Each chained process home runs, as a graph: a red step is one whose alert is firing, so that is where to dig.',
  editable: true,
  fiscalYearStartMonth: 0,
  graphTooltip: 0,
  id: null,
  links: [
    { title: 'CALL', type: 'link', url: '/d/call', icon: 'dashboard' },
    { title: 'MAILBOX', type: 'link', url: '/d/mailbox', icon: 'dashboard' },
  ],
  liveNow: false,
  panels: panelDefaults.hardenAll([
    panels.graph(pipeline) + {
      id: i + 1,
      gridPos: { x: 0, y: i * height, w: 24, h: height },
    }
    for i in std.range(0, std.length(defs.pipelines) - 1)
    for pipeline in [defs.pipelines[i]]
  ]),
  refresh: '1m',
  revision: 1,
  schemaVersion: 39,
  tags: ['pipelines'],
  templating: { list: [] },
  time: { from: 'now-1h', to: 'now' },
  timepicker: {},
  timezone: '',
  title: 'PIPELINES',
  uid: 'pipelines',
  version: 1,
  weekStart: '',
}
