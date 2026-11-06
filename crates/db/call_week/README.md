# call_week_repo

Home's record of the weekly call's weeks (paulgsc/some-ui#2635, ST36): each
week `file_host`'s `call_weeks` worker took from `paulgsc/testing`'s `call/`
folder, kept once by its first day, byte for byte with its SHA-256
(`call_week`). Append-only: the repository keeps only the last few weeks, and
home keeps every one. The weeks say what merged in two public repositories,
so the table is not subject-scoped.

The rules are in `src/lib.rs` and in
`migrations/20261106000100_create_call_week.up.sql`.
