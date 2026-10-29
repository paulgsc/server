# Storage: what home keeps, and how fast it grows

STORAGE1 (#542), under `[EPIC][STORAGE]` (#541). Home's database only ever
grows: most of what it keeps is append-only, `auto_vacuum` is off, and the
phone, the Jobs mailbox, voice notes and the game feed all end up there. This
measures that growth cheaply. It is an anomaly detector, not a capacity plan:
it says where an expensive question is worth asking.

| Where | What |
|---|---|
| `migrations/20261029000100_create_storage_sample.up.sql` | `storage_sample`, append-only: one row set per UTC day. |
| `crates/db/storage` (`storage_repo`) | The reads (`footprint`, `btrees`, `btree_bytes`), the day's write (`record`) and the windows (`figures`). |
| `apps/servers/file_host/src/storage.rs` | The hourly check and its pass; the 10 s sweep's sizes. |
| `apps/servers/file_host/src/metrics/storage.rs` | Every gauge below. |
| `infra/grafana/dashboards/storage.jsonnet` | The STORAGE row (`lib/storage-panels.libsonnet`). |

## What counts as "what home keeps"

The one SQLite file `file_host` opens (`DATABASE_URL`; on home
`/app/dbs/hopium.db`), measured four ways:

- **Each table and each index, by name** (`kind` `table` or `index`): every
  page its b-tree owns, interior, leaf and overflow pages alike, read from
  SQLite's `dbstat` table. A page's unused space counts toward its b-tree,
  because the file holds it. The schema table's own b-tree is
  `sqlite_schema`. These plus the free pages are the whole file, and
  `storage_repo`'s `dbstat_is_in_this_build` test checks that they add up.
- **The database** (`kind` `database`): `page_count × page_size`, the file's
  length. Growth and runway are read from this.
- **The WAL** (`kind` `wal`): the `-wal` file's length when the sample is
  taken, 0 if there is none. Kept apart, because the WAL rises between
  checkpoints and falls back to 64 MiB at most after one
  (`db::WAL_SIZE_LIMIT_BYTES`, STORAGE2 #543). Counted as growth, it would add
  checkpoint noise to the rate.
- **Free disk** (`kind` `disk_free`): bytes an unprivileged writer can still
  use on the database's filesystem (`statvfs`: `f_bavail × f_frsize`).

Not counted here: the other volumes on the same disk (the Whisper and Ollama
models, Metabase), Docker images, and the VPS-fit verdict, which belong to
#154; the Jobs mailbox's D1 store (some-ui#2204); the phone's own store. Free
disk is the one figure those can move, so a model download shows up as a drop
in free disk and a shorter runway, while the database's own rows stay the same.

## The cadence, and why

- **Every 10 s, O(1)** (`metrics::periodic`): `sqlite_db_bytes`,
  `sqlite_freelist_bytes` and `sqlite_wal_bytes`. Three reads of the file's
  header (`page_count`, `page_size`, `freelist_count`, about 0.15 ms) and
  one `stat` of the WAL. A tick that waits more than a second for a pooled
  connection skips, and the gauges keep their last values.
- **Once per UTC day, checked every hour** (`storage::spawn`). The first check
  runs at start, then one runs every hour. Each check reads one row: is today
  held? If it is, the check stops there. If not, it samples. So a restart never
  samples a day twice (the primary key `(day, kind, name)` and the
  in-transaction check in `storage_repo::record` refuse it anyway), and a host
  that was down at any given hour still samples that day. Daily, because the
  windows are measured in days, and about 80 rows a day (a few MB a year)
  keeps all of the history. Hourly at most, because the issue says so: a
  sample costs a full read of the file.
- **The windows, every hour** (`storage::refresh_figures`). After each check
  the server reads the newest set and the oldest set inside each window, so a
  restart republishes the figures without sampling again. That is three range
  reads on the primary key, whatever the length of the history.

## What the daily pass costs (RE7)

RE7: idle maintenance has its own budget. A pass reads every page of the file
once, so its cost scales with the file's size, not with what changed.

**Measured.** The epic's recon (#541) measured `dbstat` at about 356 ms per GB,
warm, on a throwaway database built from all migrations. This implementation
was measured on 2026-10-29: a release build reading a 1.09 GB file (1 GiB of
1 MiB blobs, plus 200,000 small rows and an index), warm, took 270 to 340 ms
for the whole pass. The 10 s header read took about 0.15 ms. Cold, a pass is
bound by how fast the disk reads the whole file once.

**The budget:**

- **At most one full pass per UTC day.** A check that finds the day held
  reads one row.
- **One b-tree per step.** Each step takes a connection from the pool and
  gives it back, and yields to the runtime between steps. The pass never
  holds more than one of the pool's five connections, and it never holds one
  across steps. It only reads, so in WAL mode it never blocks a writer. While
  a step runs, its snapshot holds the WAL's checkpoint back.
- **A deadline, `storage::PASS_DEADLINE` (5 minutes).** It is checked between
  steps, the waker's pattern, and once more before the write: a pass past it
  stops, writes nothing, and sets `storage_sample_failing` to 1, and the next
  hour tries again. A step that started in time can run past the deadline by
  its own read time, which is at most the largest b-tree's; the pass then
  writes nothing. At the warm rate, 5 minutes covers roughly
  800 GB. Cold, at 100 MB/s, it covers about 30 GB, which is near where
  `voice_note`'s migration says the audio moves out (about 20 GB). The STORAGE
  row draws the last pass against the deadline, so a pass nearing its bound
  shows there before it fails.
- **It fails nothing else.** A failed check, whether storage, filesystem or
  deadline, is logged at `error`, counted, and left. Shutdown stops it between
  steps or before the write, and then nothing is written.

A day's set is not one snapshot. Each b-tree is read in its own, and a set
is taken within one pass's seconds. For a daily growth figure, that is close
enough.

## How the figures reach Grafana

Grafana has only the Prometheus datasource, and Prometheus keeps 200 hours.
A 90-day window can't be read from it, so the server reads the windows from
`storage_sample` and exports the results. The panels only divide. There is no
SQLite datasource (the epic decided against one).

- **A window of N days** runs from the oldest set inside it (`day ≥ newest −
  N days`) to the newest set. The rate is the database's change over that
  span, per day. Until the history covers the window, the span is shorter
  than N. `storage_growth_span_days` says how long it is, and while the
  window reaches no earlier day there is no rate.
- **Each table's growth** is its change over the 7-day window. A b-tree that
  is new inside the window grew by all it holds. The share panel divides
  each b-tree's growth by the total, counting a shrink as 0.
- **Runway** is free disk ÷ the 7-day rate: a straight line. It is −1
  ("not growing") when the file did not grow over the week.
- **A figure with no value is NaN**, which the panels show as "no data", not
  0: a rate and each table's growth while the window reaches no earlier day
  (one day held, or the host down for longer than the window), the runway
  while there is no rate. It is set to NaN, never left alone, so a figure
  that stops being known does not keep its last value.

| Series | Set by | Says |
|---|---|---|
| `sqlite_db_bytes` | 10 s sweep | The file's length now. |
| `sqlite_freelist_bytes` | 10 s sweep | Pages the file holds and no b-tree uses. |
| `sqlite_wal_bytes` | 10 s sweep | The `-wal` file's length now. |
| `storage_sample_bytes{kind,name}` | hourly | Every row of the newest set. |
| `storage_table_growth_7d_bytes{kind,name}` | hourly | Each table's and index's change over the 7-day window; NaN while the span is 0. |
| `storage_growth_bytes_per_day{window}` | hourly | The database's rate over `7d` and `90d`; NaN while the span is 0. |
| `storage_growth_span_days{window}` | hourly | The days each window covers so far. |
| `storage_runway_days` | hourly | Free disk ÷ the 7-day rate; −1 not growing; NaN with no rate. |
| `storage_sample_timestamp_seconds` | hourly | When the newest set was taken. |
| `storage_sample_failing` | hourly | 1 if the last check failed. |
| `storage_sample_last_pass_duration_seconds` | each pass | How long the last pass that measured took. |
| `storage_sample_pass_deadline_seconds` | once | `PASS_DEADLINE`. |

Labels are a kind and a table's or index's name. The schema bounds them
(about 80), and they are never anything a person wrote. A table dropped by a
migration keeps its last series until the restart that deploys the migration.

## `dbstat` in this build

`dbstat` is an SQLite virtual table that exists only when SQLite is compiled
with `SQLITE_ENABLE_DBSTAT_VTAB`. sqlx's `sqlite` feature builds
`libsqlite3-sys` (0.30.1) with its bundled SQLite, and that crate's
`build.rs` sets the flag. The release image builds from the same lockfile,
so it links the same SQLite. This was verified on this build by
`storage_repo`'s `dbstat_is_in_this_build` test, which also checks that the
b-trees plus the free pages make up the whole file.

No fallback is built. If a later build lacks `dbstat`, that test fails first.
On home, the pass would then fail with `no such table: dbstat` and FAILING
would read 1; it would not write partial numbers. The fallback the issue names
would replace the per-table rows: `SUM(size)` on the tables that record a
size (`voice_note`, the mailbox tables), `SUM(length(body))` on
`game_feed_body` (the game feed keeps each body once, so `game_feed_response`'s
`size` counts a body once per fetch, not the bytes held), plus the database's
bytes.

## Deliberately not measured

- **No per-request byte accounting.** Bytes per unit of work come from
  dividing a table's growth by the counters that already exist
  (`game_feed_fetches_total`, `mailbox_poll_items_archived_total`, voice-note
  `PUT`s in `http_requests_total`, `transcribe_attempts_total`), not from
  measuring each write.
- **No forecast beyond the straight-line runway.** No fitted curve, no
  seasonality, no second derivative. The 7-day rate against the 90-day rate
  is the only "is this new" signal.
- **No alerts.** Not in this story.
- **No budget shares per table or family.** The epic decided against them.
  STORAGE3 states each family's growth model in the assurance ledger (RE9),
  and these samples are its `measured` evidence.
- **No per-subject sizes.** `storage_sample` is nobody's record
  (`docs/identity.md` invariant 1 classifies it as not subject-scoped).
- **Nothing outside the database file**, other than free disk (above).
