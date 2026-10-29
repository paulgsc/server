# storage_repo

What home keeps, by table, once a day: `storage_sample`, one append-only row
set per UTC day (each table and index from `dbstat`, the whole file, the WAL
and free disk), and the growth figures read from it (the 7- and 90-day rates,
each table's growth, the runway). Bytes per table and file, not subject-scoped.

The rules are in `src/lib.rs`, in
`migrations/20261029000100_create_storage_sample.up.sql`, and in
`docs/storage.md`.
