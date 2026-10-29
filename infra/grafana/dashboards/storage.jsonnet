// storage.jsonnet
//
// STORAGE: what home's database keeps, how fast it grows, which tables grow
// it, and how long until the disk fills at that rate (STORAGE1,
// paulgsc/server#542; docs/storage.md). Panel layer:
// lib/storage-panels.libsonnet.
local panels = import 'lib/storage-panels.libsonnet';
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
  description: "What home's database keeps, by table, and how fast it grows.",
  editable: true,
  fiscalYearStartMonth: 0,
  graphTooltip: 1,
  id: null,
  links: [],
  liveNow: false,
  panels: panelDefaults.hardenAll([
    row('Storage — what home keeps', 0, 1),
    panels.database + at(0, 1, 3, 4) + { id: 2 },
    panels.wal + at(3, 1, 3, 4) + { id: 3 },
    panels.growthShort + at(6, 1, 3, 4) + { id: 4 },
    panels.growthLong + at(9, 1, 3, 4) + { id: 5 },
    panels.regime + at(12, 1, 3, 4) + { id: 6 },
    panels.runway + at(15, 1, 3, 4) + { id: 7 },
    panels.history + at(18, 1, 2, 4) + { id: 8 },
    panels.sinceSample + at(20, 1, 2, 4) + { id: 9 },
    panels.failing + at(22, 1, 2, 4) + { id: 10 },
    panels.size + at(0, 5, 16, 8) + { id: 11 },
    panels.diskFree + at(16, 5, 8, 8) + { id: 12 },
    panels.growthShare + at(0, 13, 12, 10) + { id: 13 },
    panels.largest + at(12, 13, 12, 10) + { id: 14 },
    panels.passCost + at(0, 23, 18, 6) + { id: 15 },
    panelDefaults.livenessPanel('FILE_HOST SCRAPE', ['file_host'], 16, { x: 18, y: 23, w: 6, h: 6 }),
  ]),
  refresh: '1m',
  revision: 1,
  schemaVersion: 39,
  tags: ['storage', 'sqlite'],
  templating: { list: [] },
  // Prometheus keeps 200 h, so a week is the longest range with data
  // throughout; the 7- and 90-day windows are the server's own gauges.
  time: { from: 'now-7d', to: 'now' },
  timepicker: {},
  timezone: '',
  title: 'STORAGE',
  uid: 'storage',
  version: 1,
  weekStart: '',
}
