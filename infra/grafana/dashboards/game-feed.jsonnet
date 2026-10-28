// game-feed.jsonnet
//
// GAME FEED: home following the week's NFL games on ESPN (paulgsc/some-ui#2076,
// GWB2). Panel layer: lib/game-feed-panels.libsonnet.
local panels = import 'lib/game-feed-panels.libsonnet';
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
  description: 'The game feed: whether home is following the games of the week and keeping what ESPN says.',
  editable: true,
  fiscalYearStartMonth: 0,
  graphTooltip: 1,
  id: null,
  links: [],
  liveNow: false,
  panels: panelDefaults.hardenAll([
    row('Home following the games', 0, 1),
    panels.following + at(0, 1, 4, 4) + { id: 2 },
    panels.sinceLastLook + at(4, 1, 5, 4) + { id: 3 },
    panels.failing + at(9, 1, 5, 4) + { id: 4 },
    panels.games + at(14, 1, 5, 4) + { id: 5 },
    panels.playsToday + at(19, 1, 5, 4) + { id: 6 },
    panels.fetchesByKind + at(0, 5, 24, 8) + { id: 7 },
  ]),
  refresh: '1m',
  revision: 1,
  schemaVersion: 39,
  tags: ['nfl', 'game-feed'],
  templating: { list: [] },
  time: { from: 'now-7d', to: 'now' },
  timepicker: {},
  timezone: '',
  title: 'GAME FEED',
  uid: 'game-feed',
  version: 1,
  weekStart: '',
}
