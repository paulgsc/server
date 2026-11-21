// outbound.jsonnet
//
// OUTBOUND: every request home makes to the outside (paulgsc/some-ui#3330),
// by caller, host and outcome, and every container's network beside it.
// Panel layer: lib/outbound-panels.libsonnet.
local panels = import 'lib/outbound-panels.libsonnet';
local panelDefaults = import 'lib/panel-defaults.libsonnet';

local row(title, y, id) = {
  type: 'row',
  title: title,
  collapsed: false,
  gridPos: { h: 1, w: 24, x: 0, y: y },
  id: id,
  panels: [],
};

local at(x, y, w, h) = { gridPos: { x: x, y: y, w: w, h: h } };

{
  annotations: {
    list: [
      {
        builtIn: 1,
        datasource: { type: 'grafana', uid: '-- Grafana --' },
        enable: true,
        hide: true,
        iconColor: 'rgba(0, 211, 255, 1)',
        name: 'Annotations & Alerts',
        type: 'dashboard',
      },
    ],
  },
  description: 'What home calls on the outside, how often, and how it went; and what every container sends and receives.',
  editable: true,
  fiscalYearStartMonth: 0,
  graphTooltip: 1,
  id: null,
  links: [],
  liveNow: false,
  panels: panelDefaults.hardenAll([
    row('What file_host asks the outside', 0, 1),
    panels.requestsPerMinute + at(0, 1, 5, 4) + { id: 2 },
    panels.requestsToday + at(5, 1, 5, 4) + { id: 3 },
    panels.failedToday + at(10, 1, 5, 4) + { id: 4 },
    panels.p95 + at(15, 1, 4, 4) + { id: 5 },
    panels.bytesToday + at(19, 1, 5, 4) + { id: 6 },
    panels.byCaller + at(0, 5, 12, 9) + { id: 7 },
    panels.byHost + at(12, 5, 12, 9) + { id: 8 },
    row('How it went', 14, 9),
    panels.failures + at(0, 15, 12, 8) + { id: 10 },
    panels.latency + at(12, 15, 12, 8) + { id: 11 },
    panels.bytes + at(0, 23, 18, 7) + { id: 12 },
    panelDefaults.livenessPanel('FILE_HOST SCRAPE', ['file_host'], 17, { x: 18, y: 23, w: 6, h: 7 }),
    row('Every container\'s network (cadvisor)', 30, 13),
    panels.containersNamed + at(0, 31, 4, 8) + { id: 14 },
    panels.containerTransmit + at(4, 31, 10, 8) + { id: 15 },
    panels.containerReceive + at(14, 31, 10, 8) + { id: 16 },
  ]),
  refresh: '1m',
  revision: 1,
  schemaVersion: 39,
  tags: ['outbound', 'network'],
  templating: { list: [] },
  time: { from: 'now-24h', to: 'now' },
  timepicker: {},
  timezone: '',
  title: 'OUTBOUND',
  uid: 'outbound',
  version: 1,
  weekStart: '',
}
