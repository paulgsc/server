// transcribe.jsonnet
//
// TRANSCRIBE: home hearing the owner's voice notes. file_host's `transcribe`
// worker transcribes each note the phone sends home on home's CPU
// (paulgsc/server#462). Panel layer: lib/transcribe-panels.libsonnet.
local panels = import 'lib/transcribe-panels.libsonnet';
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
  description: 'Voice-note transcription: whether home is hearing the notes the phone sends it.',
  editable: true,
  fiscalYearStartMonth: 0,
  graphTooltip: 1,
  id: null,
  links: [],
  liveNow: false,
  panels: panelDefaults.hardenAll([
    row('Home hearing voice notes', 0, 1),
    panels.hearing + at(0, 1, 4, 4) + { id: 2 },
    panels.sinceLastLook + at(4, 1, 5, 4) + { id: 3 },
    panels.failing + at(9, 1, 5, 4) + { id: 4 },
    panels.heardToday + at(14, 1, 5, 4) + { id: 5 },
    panels.notHeardToday + at(19, 1, 5, 4) + { id: 6 },
    panels.attemptsByOutcome + at(0, 5, 12, 8) + { id: 7 },
    panels.attemptDuration + at(12, 5, 12, 8) + { id: 8 },
  ]),
  refresh: '1m',
  revision: 1,
  schemaVersion: 39,
  tags: ['voice', 'transcribe'],
  templating: { list: [] },
  time: { from: 'now-7d', to: 'now' },
  timepicker: {},
  timezone: '',
  title: 'TRANSCRIBE',
  uid: 'transcribe',
  version: 1,
  weekStart: '',
}
