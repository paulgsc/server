// storage-panels.libsonnet
//
// The STORAGE row: what home's database keeps, how fast it grows, which
// tables grow it, and how long until the disk fills at that rate
// (STORAGE1, paulgsc/server#542; docs/storage.md). Every series here comes
// from apps/servers/file_host/src/metrics/storage.rs; labels are a kind and
// a table's or index's name, never anything a person wrote.
//
// Two sources, on purpose. `sqlite_*_bytes` are the 10 s sweep, so the size
// chart is only as long as Prometheus keeps (200 h). The growth windows are
// longer than that (7 and 90 days), so the server reads them from its own
// daily `storage_sample` table and exports the results as `storage_*`
// gauges; these panels only divide. A rate is NaN while the window reaches
// no earlier day, so it reads "no data", not 0.

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

local bars(title, description, target, unit) = {
  title: title,
  description: description,
  type: 'bargauge',
  datasource: ds,
  targets: [target { instant: true, legendFormat: '{{name}}', refId: 'A' }],
  fieldConfig: {
    defaults: {
      unit: unit,
      min: 0,
      color: { mode: 'thresholds' },
      thresholds: { mode: 'absolute', steps: informational },
    },
    overrides: [],
  },
  options: {
    displayMode: 'gradient',
    orientation: 'horizontal',
    showUnfilled: true,
    valueMode: 'color',
    namePlacement: 'top',
    reduceOptions: { calcs: ['lastNotNull'], values: false },
  },
};

{
  database: stat(
    'DATABASE',
    'The database file now: page_count × page_size, read from its header every 10 s. The WAL is separate.',
    { expr: 'sqlite_db_bytes' },
    'bytes',
    informational,
  ),

  wal: stat(
    'WAL',
    'The -wal file now. It rises between checkpoints and falls back to 64 MiB at most after one (journal_size_limit), so it is never counted as growth.',
    { expr: 'sqlite_wal_bytes' },
    'bytes',
    informational,
  ),

  growthShort: stat(
    'GROWTH / DAY, 7D',
    "The database file's change per day, newest daily sample against the oldest one inside the last 7 days. No data until two days are sampled.",
    { expr: 'storage_growth_bytes_per_day{window="7d"}' },
    'bytes',
    informational,
  ),

  growthLong: stat(
    'GROWTH / DAY, 90D',
    'The same over the last 90 days, or over all the history home holds until it holds 90 days (HISTORY says how many).',
    { expr: 'storage_growth_bytes_per_day{window="90d"}' },
    'bytes',
    informational,
  ),

  regime: stat(
    '7D ÷ 90D',
    'The short rate over the long one: near 1 is growth as usual. Well above 1, something started keeping more this week; that is where a question is worth asking. Not an alert.',
    { expr: 'storage_growth_bytes_per_day{window="7d"} / ignoring(window) storage_growth_bytes_per_day{window="90d"}' },
    'none',
    [{ color: 'green', value: null }, { color: 'orange', value: 2 }, { color: 'red', value: 4 }],
    decimals=2,
  ),

  runway: stat(
    'RUNWAY',
    "Free disk on the database's filesystem ÷ the 7-day rate: a straight line, no forecast. NOT GROWING when the file did not grow over the week.",
    { expr: 'storage_runway_days' },
    'd',
    [{ color: 'red', value: null }, { color: 'orange', value: 30 }, { color: 'green', value: 90 }],
    [{ type: 'value', options: { '-1': { text: 'NOT GROWING', color: 'green' } } }],
    decimals=0,
  ),

  history: stat(
    'HISTORY',
    'Days of daily samples the 90-day window covers so far. The long rate means what it says once this reaches 90.',
    { expr: 'storage_growth_span_days{window="90d"}' },
    'd',
    informational,
    decimals=0,
  ),

  sinceSample: stat(
    'SINCE LAST SAMPLE',
    'Time since the newest daily sample was taken. Home checks every hour and samples once per UTC day, so up to about 25 h is normal.',
    { expr: 'time() - storage_sample_timestamp_seconds' },
    's',
    [{ color: 'green', value: null }, { color: 'red', value: 26 * 3600 }],
  ),

  failing: stat(
    'FAILING',
    'Whether the last hourly check failed: 0 is healthy. A failed or timed-out pass writes nothing and the next hour tries again; the log says why.',
    { expr: 'storage_sample_failing' },
    'none',
    [{ color: 'green', value: null }, { color: 'red', value: 1 }],
  ),

  size: series(
    'The database, every 10 s',
    'The file, its free pages (held, reused before the file grows) and the WAL. As long as Prometheus keeps (200 h); the long windows are the stats above.',
    [
      { expr: 'sqlite_db_bytes', legendFormat: 'database', refId: 'A' },
      { expr: 'sqlite_freelist_bytes', legendFormat: 'free pages', refId: 'B' },
      { expr: 'sqlite_wal_bytes', legendFormat: 'wal', refId: 'C' },
    ],
    'bytes',
  ),

  diskFree: series(
    'Free disk, daily',
    "Free bytes on the database's filesystem at each daily sample. The same disk holds the models and Metabase, so a drop here is not always the database.",
    [{ expr: 'storage_sample_bytes{kind="disk_free"}', legendFormat: 'free', refId: 'A' }],
    'bytes',
  ),

  growthShare: bars(
    'Share of growth, 7 days',
    "Each table's and index's share of what was added over the 7-day window, from the daily samples. A b-tree that shrank counts as 0.",
    { expr: 'topk(12, clamp_min(storage_table_growth_7d_bytes, 0) / scalar(sum(clamp_min(storage_table_growth_7d_bytes, 0))))' },
    'percentunit',
  ),

  largest: bars(
    'Largest tables and indexes',
    'Bytes each b-tree holds at the newest daily sample, overflow pages included (dbstat).',
    { expr: 'topk(12, storage_sample_bytes{kind=~"table|index"})' },
    'bytes',
  ),

  passCost: series(
    'Daily pass, its cost against its bound',
    'How long the last sampling pass took, against the deadline it stops at. It reads every page once, about 0.3 s per GB warm (docs/storage.md).',
    [
      { expr: 'storage_sample_last_pass_duration_seconds', legendFormat: 'last pass', refId: 'A' },
      { expr: 'storage_sample_pass_deadline_seconds', legendFormat: 'deadline', refId: 'B' },
    ],
    's',
  ),
}
