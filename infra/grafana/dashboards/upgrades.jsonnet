// upgrades.jsonnet
//
// UPGRADES: the box's nightly NixOS + home-manager upgrade (paulgsc/dotfiles
// nixos/auto-upgrade), as its reporter judged it: did last night's run land
// on main, both halves, with the capture working; what did it print; and how
// many nights out of the last 7/30/90 were good. Panel layer:
// lib/upgrade-panels.libsonnet.
local panels = import 'lib/upgrade-panels.libsonnet';
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
  description: 'The nightly NixOS + home-manager upgrade: last night\'s verdict, what it printed, and good nights over 7/30/90 days.',
  editable: true,
  fiscalYearStartMonth: 0,
  graphTooltip: 1,
  id: null,
  links: [
    {
      title: 'Deployments (run-by-run record)',
      type: 'link',
      url: 'https://github.com/paulgsc/dotfiles/deployments/nixos',
      targetBlank: true,
    },
    {
      title: 'upgrade-log issues',
      type: 'link',
      url: 'https://github.com/paulgsc/dotfiles/issues?q=is%3Aissue+label%3Aupgrade-log',
      targetBlank: true,
    },
  ],
  liveNow: false,
  panels: panelDefaults.hardenAll([
    row('Last night', 0, 1),
    panels.verdict + at(0, 1, 3, 4) + { id: 2 },
    panels.sinceRun + at(3, 1, 4, 4) + { id: 3 },
    panels.sinceGood + at(7, 1, 4, 4) + { id: 4 },
    panels.findings + at(11, 1, 3, 4) + { id: 5 },
    panels.nearMisses + at(14, 1, 4, 4) + { id: 6 },
    panels.github + at(18, 1, 3, 4) + { id: 7 },
    panelDefaults.livenessPanel('NODE SCRAPE', ['node'], 8, { x: 21, y: 1, w: 3, h: 4 }),
    panels.checks + at(0, 5, 12, 5) + { id: 9 },
    panels.deployed + at(12, 5, 8, 5) + { id: 10 },
    panels.metricsFile + at(20, 5, 4, 5) + { id: 11 },
    row('What it printed', 10, 12),
    panels.findingLines + at(0, 11, 18, 9) + { id: 13 },
    panels.report + at(18, 11, 6, 9) + { id: 14 },
    row('Over time', 20, 15),
    panels.goodNights + at(0, 21, 12, 6) + { id: 16 },
    panels.nightCounts + at(12, 21, 12, 6) + { id: 17 },
    panels.timeline + at(0, 27, 24, 8) + { id: 18 },
  ]),
  refresh: '1m',
  revision: 1,
  schemaVersion: 39,
  tags: ['upgrades', 'nixos', 'dotfiles'],
  templating: { list: [] },
  time: { from: 'now-7d', to: 'now' },
  timepicker: {},
  timezone: '',
  title: 'UPGRADES',
  uid: 'upgrades',
  version: 1,
  weekStart: '',
}
