// pipeline-panels.libsonnet
//
// The PIPELINES dashboard's graphs (server#692): each process of
// lib/pipelines.json as a node graph, its nodes the ledger's segments and
// each node's color the alerts firing for it: red while one of its alerts
// fires, orange while one is pending, green while none is, gray when it has
// no signal of its own (a step someone else runs). Every color comes from
// ALERTS, so the graph says what Slack says, placed on the pipeline, and a
// red node is where to dig.
//
// The node graph panel takes no field config, and the browser names a
// table's value column `Value #<refId>`, so everything it reads is a label
// the query sets: `id`, `title`, `subtitle`, `mainstat` (the word in the
// circle), `color` (an HTML color) and `detail__*` (shown when a node is
// clicked). A node's branches are joined with `or on()`, which takes the
// next only when the one before has nothing, so each node is one row.

local ds = { type: 'prometheus', uid: 'prometheus' };

local colors = { firing: '#F2495C', pending: '#FF9830', ok: '#73BF69', none: '#8E8E8E' };

local quote(s) = std.strReplace(std.strReplace(s, '\\', '\\\\'), '"', '\\"');

// label_replace that sets `label` to a fixed `value` on every series.
local setLabel(expr, label, value) =
  'label_replace(' + expr + ', "' + label + '", "' + quote(value) + '", "", "")';

local stateOf(expr, word, color) = setLabel(setLabel(expr, 'mainstat', word), 'color', color);

local alerts(node, state) =
  std.join(' or ', ['ALERTS{alertstate="' + state + '",' + matcher + '}' for matcher in node.alerts]);

local state(node) =
  if std.length(node.alerts) == 0 then stateOf('vector(0)', 'not ours', colors.none)
  else '(' + std.join(' or on() ', [
    stateOf('clamp_max(count(' + alerts(node, 'firing') + '), 1)', 'FIRING', colors.firing),
    stateOf('clamp_max(count(' + alerts(node, 'pending') + '), 1)', 'pending', colors.pending),
    stateOf('vector(0)', 'ok', colors.ok),
  ]) + ')';

local watched(node) =
  if std.length(node.alerts) == 0 then 'nothing of ours: it shows through the next step'
  else std.join(', ', [std.strReplace(std.strReplace(m, 'alertname=', ''), '"', '') for m in node.alerts]);

local nodeExpr(node) =
  std.foldl(
    function(expr, kv) setLabel(expr, kv[0], kv[1]),
    [
      ['id', node.id],
      ['title', node.title],
      ['subtitle', node.subtitle],
      ['detail__alerts', watched(node)],
      ['detail__dashboard', if std.objectHas(node, 'dashboard') then '/d/' + node.dashboard else 'none'],
    ],
    state(node),
  );

local edgeExpr(edge) =
  setLabel(setLabel(setLabel('vector(1)', 'id', edge[0] + '-' + edge[1]), 'source', edge[0]), 'target', edge[1]);

{
  graph(pipeline):: {
    title: pipeline.title,
    description: 'Each step of ' + pipeline.title + ' (the ledger process `' + pipeline.process + '`). Red: one of its alerts is firing, and Slack has said so. Orange: one is pending. Green: none is. Gray: a step someone else runs, with no signal of its own; it shows through the next step of ours. Click a node for the alerts it watches and its dashboard.',
    type: 'nodeGraph',
    datasource: ds,
    targets: [
      {
        refId: 'nodes',
        expr: std.join('\nor ', [nodeExpr(node) for node in pipeline.nodes]),
        instant: true,
        range: false,
        format: 'table',
      },
      {
        refId: 'edges',
        expr: std.join('\nor ', [edgeExpr(edge) for edge in pipeline.edges]),
        instant: true,
        range: false,
        format: 'table',
      },
    ],
    options: { nodes: {}, edges: {}, zoomMode: 'cooperative' },
  },
}
