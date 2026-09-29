-- #381 (LTY-EXEC) and #328 (LTY-SRV4): one recorded run of a LeetType round's
-- program, per variant and per constraint set, for the round version it was
-- recorded against.
--
-- WHAT A ROW IS. The `RunResult` (`@some-ui/leetype`'s
-- `lib/leetype/run-result`) that `leetype_runner` produced when it compiled
-- one variant of a round -- `A` itself, or `A + d` for the member of `D` at
-- position i ('d0' ... 'd4') -- with the round's `harness` appended, and ran
-- it once at the bounds of `constraintDiff.before` or `constraintDiff.after`.
-- The design, and the options it rejected, are in
-- `apps/servers/file_host/docs/leetype-execution.md`.
--
-- WHO WRITES IT. Only the offline `record-leetype-runs` command, on a machine
-- with `rustc`. The server never compiles or executes anything: the route
-- `GET /leetype/rounds/:id/runs` reads these rows, and `dump-leetype-snapshot`
-- writes them into the static build's snapshot. So a live answer and a static
-- one come from the same runner and the same rows.
--
-- WHY `content_hash` IS PART OF THE ROW. A round's body can change (a
-- re-import, the operator's write route) without anyone re-recording. The
-- runs belong to the bytes they were recorded from, so every read joins on
-- the round's *current* `content_hash`, and a run recorded for an older body
-- is never served: it stays here, unread, until the next recording replaces
-- it. A round's runs are replaced as a set, in one transaction, for one hash.
--
-- NOT HERE, ON PURPOSE: any complexity claim (no class, no Θ, no
-- "admissible" -- paulgsc/server#381's never #3), anything about who asked
-- (a run is corpus-wide, never a subject's), and any link to the ledger, the
-- sampler or the study nudge (never #5). `privacy.rs` classifies the table
-- NOT_SUBJECT_SCOPED.
--
-- Columns:
--
--   round_id     -- `leetype_round.id`; the runs go with the round.
--   content_hash -- the round's `content_hash` when these runs were recorded.
--   variant      -- 'A', or 'd<i>' for `diffOptions[i]` (i from 0).
--   bounds       -- which constraint set the run's sizes came from.
--   sizes        -- JSON object, dimension -> the bound the harness was given.
--   result       -- the `RunResult`, as JSON.
--   recorded_at  -- ISO-8601 UTC. For the operator; never served or dumped, so
--                   re-recording an unchanged round moves nothing downstream
--                   but this column.
CREATE TABLE leetype_round_run (
    round_id     TEXT NOT NULL REFERENCES leetype_round(id) ON DELETE CASCADE,
    content_hash TEXT NOT NULL,
    variant      TEXT NOT NULL CHECK (variant = 'A' OR variant GLOB 'd[0-9]'),
    bounds       TEXT NOT NULL CHECK (bounds IN ('before', 'after')),
    sizes        TEXT NOT NULL,   -- JSON object
    result       TEXT NOT NULL,   -- RunResult JSON
    recorded_at  TEXT NOT NULL,   -- ISO-8601 UTC
    PRIMARY KEY (round_id, variant, bounds)
);
