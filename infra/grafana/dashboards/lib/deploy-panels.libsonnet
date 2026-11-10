// deploy-panels.libsonnet
//
// The DEPLOY row: home's on-demand deploy (README.md, "Deploying to home";
// apps/servers/file_host/src/deploy.rs), and the footprint of the pattern
// itself, so its growth can be projected. Every series comes from
// apps/servers/file_host/src/metrics/deploy.rs. The deploy's own totals are
// read from its state.json and exported as gauges that only rise, so
// increase() reads them as counters.
//
// The footprint is what this pattern costs: the clients' asks (compat
// checks, by answer), the pulls they caused (each one request against
// Docker Hub's anonymous limit of 100 per 6 h) and their bytes, and the
// migration backups a deploy leaves on the database's disk.

local ds = { type: 'prometheus', uid: 'prometheus' };

local stat(title, description, target, unit, steps, mappings=[], decimals=null) = {
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
    } + (if decimals != null then { decimals: decimals } else {}),
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

local informational = [{ color: 'blue', value: null }];

local series(title, description, targets, unit) = {
  title: title,
  description: description,
  type: 'timeseries',
  datasource: ds,
  targets: targets,
  fieldConfig: {
    defaults: {
      unit: unit,
      min: 0,
      custom: { drawStyle: 'line', lineWidth: 1, fillOpacity: 10, spanNulls: false },
    },
    overrides: [],
  },
  options: { legend: { displayMode: 'list', placement: 'bottom' }, tooltip: { mode: 'multi' } },
};

{
  epoch: stat(
    'CONTRACT EPOCH',
    'The contract epoch the running file_host serves (apps/servers/file_host/src/contract.rs). A client built against a higher one asks home to pull.',
    { expr: 'contract_epoch' },
    'none',
    informational,
    decimals=0,
  ),

  oldest: stat(
    'OLDEST SUPPORTED',
    'The lowest client epoch the running file_host still serves. A client below it is told it is behind home (client_behind), and pulling cannot help it.',
    { expr: 'contract_oldest_supported' },
    'none',
    informational,
    decimals=0,
  ),

  latestSeen: stat(
    'NEWEST SEEN',
    "The epoch of the newest image home's last pull found. Below a client's need, home answers it `unavailable` for six hours rather than pulling again.",
    { expr: 'home_deploy_latest_seen_epoch' },
    'none',
    informational,
    decimals=0,
  ),

  lastExit: stat(
    'LAST DEPLOY',
    "How home's last on-demand deploy ended: 0 deployed or nothing to do, 1 not deployed, 2 rolled back, 3 nothing ready. No data until the first one.",
    { expr: 'home_deploy_last_exit_code' },
    'none',
    [{ color: 'green', value: null }, { color: 'red', value: 1 }],
    [{
      type: 'value',
      options: {
        '0': { text: 'OK', color: 'green' },
        '1': { text: 'NOT DEPLOYED', color: 'red' },
        '2': { text: 'ROLLED BACK', color: 'red' },
        '3': { text: 'NOTHING READY', color: 'red' },
      },
    }],
  ),

  sinceRun: stat(
    'SINCE LAST DEPLOY',
    'Time since the last on-demand deploy ran. Long is normal: home deploys only when a client needs a newer server.',
    { expr: 'time() - home_deploy_last_run_timestamp_seconds' },
    's',
    informational,
  ),

  waiting: stat(
    'WANTED FOR',
    'How long a deploy-wanted marker has waited unanswered. A deploy takes minutes; past 30 the alert DeployStuck fires (the path unit is not running, or the run hung).',
    { expr: 'home_deploy_wanted_age_seconds' },
    's',
    [{ color: 'green', value: null }, { color: 'orange', value: 600 }, { color: 'red', value: 1800 }],
  ),

  backups: stat(
    'BACKUPS',
    'Migration backups held in ${STORAGE}/backups: one per start that applied a migration, the newest 5 kept (apps/servers/file_host/src/migrate.rs).',
    { expr: 'migration_backups' },
    'none',
    [{ color: 'green', value: null }, { color: 'orange', value: 6 }],
    decimals=0,
  ),

  backupBytes: stat(
    'BACKUP BYTES',
    "What the migration backups take on the database's disk: up to five whole copies of it.",
    { expr: 'migration_backup_bytes' },
    'bytes',
    informational,
  ),

  asks: series(
    'Compat asks per day, by answer',
    "What clients asked GET /compat, by home's answer: none (served), wanted (a deploy asked for), in_flight, cooldown, unavailable, refused, client_behind, cannot_request.",
    [{ expr: 'sum by (result) (increase(compat_checks_total[1d]))', legendFormat: '{{result}}', refId: 'A' }],
    'none',
  ),

  runs: series(
    'Deploy runs, cumulative, by outcome',
    "Every on-demand run home's deploy made: deployed, rolled_back, failed, no_newer_image (pulled, nothing newer to start), deferred (inside the gap between pulls).",
    [{ expr: 'home_deploy_runs', legendFormat: '{{outcome}}', refId: 'A' }],
    'none',
  ),

  pulls: series(
    'Docker Hub pulls per 6 h, against the limit',
    "Pulls in the last 6 h, the next 6 h's at the last day's rate (predict_linear), and Docker Hub's anonymous limit of 100 per 6 h per IP. Each pull is a manifest request; the bounds in deploy.rs keep a stuck client to 4 per 6 h.",
    [
      { expr: 'increase(home_deploy_pulls[6h])', legendFormat: 'last 6 h', refId: 'A' },
      { expr: 'clamp_min(predict_linear(home_deploy_pulls[1d], 6 * 3600) - home_deploy_pulls, 0)', legendFormat: 'next 6 h, projected', refId: 'B' },
      { expr: 'vector(100)', legendFormat: 'limit (anonymous)', refId: 'C' },
    ],
    'none',
  ),

  pulledBytes: series(
    'Bytes pulled, cumulative',
    'What the pulls fetched: the size of each new image, added when a pull brought one.',
    [{ expr: 'home_deploy_pulled_bytes', legendFormat: 'pulled', refId: 'A' }],
    'bytes',
  ),

  backupFootprint: series(
    'Backups against the disk',
    "The migration backups' bytes, where they go in 30 days at the last week's rate (predict_linear), and free disk on the database's filesystem.",
    [
      { expr: 'migration_backup_bytes', legendFormat: 'backups', refId: 'A' },
      { expr: 'predict_linear(migration_backup_bytes[7d], 30 * 86400)', legendFormat: 'backups in 30 d, projected', refId: 'B' },
      { expr: 'storage_sample_bytes{kind="disk_free"}', legendFormat: 'disk free', refId: 'C' },
    ],
    'bytes',
  ),
}
