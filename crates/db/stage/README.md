# stage_repo

What waits on what across home's shared stages (paulgsc/server#1091): the
`stage_want` table, one row per item a waiter is waiting on right now. Each
waiter's pass replaces its own rows whole; a stage's queue reads them to put
first what someone downstream waits on. See `docs/shared-stages.md`.
