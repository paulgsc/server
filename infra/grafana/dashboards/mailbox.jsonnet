// mailbox.jsonnet
//
// MAILBOX: the Jobs mailbox, as home sees it. The mailbox (paulgsc/some-ui
// `apps/mailbox`) is a Cloudflare Worker and D1 database where the owner's
// cloud routine leaves each day's job digest; file_host's `mailbox` poller
// reads it so the phone can take today's picks from home. Panel layer:
// lib/mailbox-panels.libsonnet.
local panels = import 'lib/mailbox-panels.libsonnet';
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
  description: 'The Jobs mailbox: whether home is reading the daily digest from it.',
  editable: true,
  fiscalYearStartMonth: 0,
  graphTooltip: 1,
  id: null,
  links: [],
  liveNow: false,
  panels: panelDefaults.hardenAll([
    row('Home reading the mailbox', 0, 1),
    panels.reading + at(0, 1, 4, 4) + { id: 2 },
    panels.sinceLastRead + at(4, 1, 5, 4) + { id: 3 },
    panels.failing + at(9, 1, 4, 4) + { id: 4 },
    panels.keptToday + at(13, 1, 5, 4) + { id: 5 },
    panels.refusedToday + at(18, 1, 6, 4) + { id: 6 },
    panels.failingByWhy + at(0, 5, 8, 8) + { id: 7 },
    panels.keptAndRefused + at(8, 5, 8, 8) + { id: 8 },
    panels.passDuration + at(16, 5, 8, 8) + { id: 9 },
  ]),
  refresh: '1m',
  revision: 1,
  schemaVersion: 39,
  tags: ['mailbox', 'jobs'],
  templating: { list: [] },
  time: { from: 'now-7d', to: 'now' },
  timepicker: {},
  timezone: '',
  title: 'MAILBOX',
  uid: 'mailbox',
  version: 1,
  weekStart: '',
}
