// call.jsonnet
//
// CALL: the weekly call home keeps and makes into a video (paulgsc/some-ui#2635,
// ST36). Panel layer: lib/call-panels.libsonnet.
local panels = import 'lib/call-panels.libsonnet';
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
  description: 'The weekly call and Playoff Delta: whether home keeps each week, and whether each becomes a video.',
  editable: true,
  fiscalYearStartMonth: 0,
  graphTooltip: 1,
  id: null,
  links: [],
  liveNow: false,
  panels: panelDefaults.hardenAll([
    row('Home keeping the weeks', 0, 1),
    panels.held + at(0, 1, 6, 4) + { id: 2 },
    panels.sinceRead + at(6, 1, 9, 4) + { id: 3 },
    panels.readFailing + at(15, 1, 9, 4) + { id: 4 },
    row('Each week made into a video', 5, 5),
    panels.waiting + at(0, 6, 6, 4) + { id: 6 },
    panels.failed + at(6, 6, 6, 4) + { id: 7 },
    panels.longestWait + at(12, 6, 6, 4) + { id: 8 },
    panels.sinceWorker + at(18, 6, 6, 4) + { id: 9 },
    panels.weeks + at(0, 10, 24, 8) + { id: 10 },
    row('The call\'s containers', 18, 11),
    panels.restarts + at(0, 19, 6, 8) + { id: 12 },
    panels.memory + at(6, 19, 18, 8) + { id: 13 },
    row('What the call keeps on the drive', 27, 14),
    panels.videos + at(0, 28, 6, 4) + { id: 15 },
    panels.work + at(6, 28, 6, 4) + { id: 16 },
    panels.sinceCacheWarning + at(12, 28, 6, 4) + { id: 17 },
    panels.driveFree + at(18, 28, 6, 4) + { id: 18 },
    panels.driveUse + at(0, 32, 24, 8) + { id: 19 },
    row('Playoff Delta, the second job', 40, 20),
    panels.pdHeld + at(0, 41, 4, 4) + { id: 21 },
    panels.pdSinceRead + at(4, 41, 4, 4) + { id: 22 },
    panels.pdReadFailing + at(8, 41, 4, 4) + { id: 23 },
    panels.pdWaiting + at(12, 41, 4, 4) + { id: 24 },
    panels.pdFailed + at(16, 41, 4, 4) + { id: 25 },
    panels.pdUnvoiced + at(20, 41, 4, 4) + { id: 26 },
    panels.pdEpisodes + at(0, 45, 24, 8) + { id: 27 },
  ]),
  refresh: '1m',
  revision: 1,
  schemaVersion: 39,
  tags: ['call'],
  templating: { list: [] },
  time: { from: 'now-7d', to: 'now' },
  timepicker: {},
  timezone: '',
  title: 'CALL',
  uid: 'call',
  version: 1,
  weekStart: '',
}
