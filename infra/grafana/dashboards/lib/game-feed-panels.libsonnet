// game-feed-panels.libsonnet
//
// The GAME FEED dashboard's panels. file_host's `game_feed` worker follows
// the week's NFL games on ESPN and keeps every response and the play log
// (docs/game-feed.md, paulgsc/some-ui#2076). Every series here comes from
// apps/servers/file_host/src/metrics/game_feed.rs; labels are fetch kinds
// only, never a game or a URL.
//
// Most of the week no game is live, so the healthy state is quiet: FOLLOWING
// says whether the deployment runs the worker, SINCE LAST LOOK whether it is
// still waking (every 5 s), so a stalled feed reads differently from a quiet
// week, and FAILING whether the last pass ended on a storage failure.

local ds = { type: 'prometheus', uid: 'prometheus' };

local stat(title, description, target, unit, steps, mappings=[]) = {
  title: title,
  description: description,
  type: 'stat',
  datasource: ds,
  targets: [target { instant: true, refId: 'A' }],
  fieldConfig: {
    defaults: {
      unit: unit,
      color: { mode: 'thresholds' },
      thresholds: { mode: 'absolute', steps: steps },
      mappings: mappings,
      noValue: 'no data',
    },
    overrides: [],
  },
  options: {
    colorMode: 'value',
    graphMode: 'none',
    justifyMode: 'center',
    orientation: 'horizontal',
    reduceOptions: { calcs: ['lastNotNull'], values: false },
    textMode: 'value',
  },
};

local series(title, description, targets, unit) = {
  title: title,
  description: description,
  type: 'timeseries',
  datasource: ds,
  targets: targets,
  fieldConfig: {
    defaults: {
      unit: unit,
      custom: { drawStyle: 'line', lineWidth: 1, fillOpacity: 10, spanNulls: false },
    },
    overrides: [],
  },
  options: { legend: { displayMode: 'list', placement: 'bottom' }, tooltip: { mode: 'multi' } },
};

{
  following: stat(
    'FOLLOWING',
    'Whether this home follows the games at all (GAME_FEED_ENABLED). OFF is a choice, not a fault.',
    { expr: 'game_feed_enabled' },
    'none',
    [{ color: 'green', value: null }],
    [{ type: 'value', options: { '0': { text: 'OFF', color: '#8E8E8E' }, '1': { text: 'ON', color: 'green' } } }],
  ),

  sinceLastLook: stat(
    'SINCE LAST LOOK',
    'Time since the worker last woke. It wakes every 5 s, fetching only what is due; past a minute, the task died.',
    { expr: 'time() - game_feed_last_attempt_timestamp_seconds' },
    's',
    [{ color: 'green', value: null }, { color: 'red', value: 60 }],
  ),

  failing: stat(
    'FAILING',
    'Whether the last pass ended on a storage failure: 0 is healthy. An ESPN refusal or no answer is not a failure here; it is kept, counted by kind, and backed off.',
    { expr: 'game_feed_pass_failing' },
    'none',
    [{ color: 'green', value: null }, { color: 'red', value: 1 }],
  ),

  games: stat(
    'GAMES FOLLOWED',
    'Games on the current scoreboard the worker follows. A game is left once its final summary is held.',
    { expr: 'game_feed_games_followed' },
    'none',
    [{ color: 'green', value: null }],
  ) { fieldConfig+: { defaults+: { decimals: 0 } } },

  playsToday: stat(
    'PLAYS ADDED, 24H',
    'Rows added to the play log in the last 24 hours, revisions included. Zero is normal on a day with no game.',
    { expr: 'sum(increase(game_feed_plays_added_total[24h]))' },
    'none',
    [{ color: 'green', value: null }],
  ) { fieldConfig+: { defaults+: { decimals: 0 } } },

  fetchesByKind: series(
    'Fetches, by what came back',
    'Per 15 minutes. scoreboard and summary are answers kept and read; rate_limited, status, not_json and not_an_object are answers kept but refused; timeout, connect and transport are no answer at all, which back off per target.',
    [{ expr: 'sum by (kind) (increase(game_feed_fetches_total[15m]))', legendFormat: '{{kind}}', refId: 'A' }],
    'none',
  ),
}
