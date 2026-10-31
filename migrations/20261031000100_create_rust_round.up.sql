-- Rust Rounds on home (paulgsc/some-ui#2234, RR3): each round a tutor wrote,
-- as the phone pasted it, and what the rust-runner recorded for each of its
-- programs. The round format is paulgsc/some-ui's `some-ui.rust-round/1`
-- (`packages/ui/rust-rounds`, RR1); the runner is `crates/rust_runner`, in
-- its own sandboxed container, and file_host only asks it, from the
-- background pass in `rust_rounds` (EX1, RX1-RX4 and RP1-RP4 in
-- apps/servers/file_host/docs/leetype-execution.md).
--
-- None of these tables names a subject: a round is a tutor's cards and code,
-- and a run is what rustc did with a program (docs/identity.md, and
-- `privacy.rs`'s NOT_SUBJECT_SCOPED, says why, and what a round can still
-- say about the owner).
--
-- ALL FOUR ARE APPEND-ONLY. No row is ever updated or deleted (the triggers
-- below abort both). A round's id is never reused (the format's own rule),
-- so another body under a held id is refused (`409`), never stored as a
-- revision and never written over: the record keeps the round the phone
-- played.
--
-- `rust_round`, one row per round:
--
--   seq          -- the order home received rounds in.
--   id           -- the round's own id (`indexing-01`), unique for good.
--   body         -- the round's JSON, byte for byte as the phone sent it
--                   (JSON whitespace around it trimmed). Read, never
--                   re-serialised.
--   content_hash -- the hex SHA-256 of `body`: a replay of the same bytes is
--                   told from another body by it.
--   received_at  -- ISO-8601 UTC, when it reached home.
CREATE TABLE rust_round (
    seq          INTEGER PRIMARY KEY AUTOINCREMENT,
    id           TEXT    NOT NULL UNIQUE,
    body         TEXT    NOT NULL,
    content_hash TEXT    NOT NULL,
    received_at  TEXT    NOT NULL
);

-- `rust_round_source`, one row per distinct program any round holds: a read
-- card's program, or a variant card's (the program of `of` with its hunk's
-- line replaced). The same bytes in two rounds are one row, recorded once.
-- It is the pass's queue: `seq` is the order programs arrived in, and the
-- pass records them in that order.
--
--   seq         -- the order it arrived in.
--   source_hash -- the hex SHA-256 of `source`.
--   source      -- the program, as the runner is sent it (`main.rs`).
--   queued_at   -- ISO-8601 UTC, when the first round holding it arrived.
CREATE TABLE rust_round_source (
    seq         INTEGER PRIMARY KEY AUTOINCREMENT,
    source_hash TEXT    NOT NULL UNIQUE,
    source      TEXT    NOT NULL,
    queued_at   TEXT    NOT NULL
);

-- `rust_round_run`, one row per program and toolchain: how its recording
-- ended. Keyed as EX2 keys LeetType's runs and RX4 the runner's: to the bytes
-- and the toolchain that made it, never to a round, a card or a tutor.
--
--   toolchain   -- the runner's (`rustc 1.97.0 (2d8144b78 2026-07-07);
--                  --edition 2024 -C opt-level=0`), as the pass asked for it.
--   source_seq  -- the program.
--   key         -- `rust_rounds_wire::recording::key(toolchain, source)`,
--                  computed by the pass from what it sent (RX4).
--   ended       -- `recorded`: the runner answered with a recording.
--                  `runner-restarted`: the runner went away while holding
--                  this program, as many times as the pass allows
--                  (`rust_rounds::pass::DROPS`), so it is kept as that,
--                  never retried again under this toolchain (RP3).
--   recording   -- for `recorded`: the runner's whole recording, as JSON.
--   rustc       -- what the phone reads (`RustcRun`, RR1's `RustcRunSchema`),
--                  as JSON; for `runner-restarted`, a `stopped` run (`exit`).
--   recorded_at -- ISO-8601 UTC, when it was kept.
CREATE TABLE rust_round_run (
    toolchain   TEXT    NOT NULL,
    source_seq  INTEGER NOT NULL REFERENCES rust_round_source (seq),
    key         TEXT    NOT NULL UNIQUE,
    ended       TEXT    NOT NULL CHECK (ended IN ('recorded', 'runner-restarted')),
    recording   TEXT,
    rustc       TEXT    NOT NULL,
    recorded_at TEXT    NOT NULL,
    PRIMARY KEY (toolchain, source_seq),
    CHECK ((ended = 'recorded') = (recording IS NOT NULL))
);

-- `rust_round_drop`, one row each time the runner went away while it held a
-- program (the connection ended after the whole request was sent): at most
-- `DROPS` per program and toolchain, since the last one is followed, in the
-- same transaction, by a `runner-restarted` run. A runner that could not be
-- reached at all is an outage and leaves no row here (RP3).
--
--   toolchain  -- as in `rust_round_run`.
--   source_seq -- the program.
--   attempt    -- 1 for the first drop, and one more for each after.
--   dropped_at -- ISO-8601 UTC.
CREATE TABLE rust_round_drop (
    toolchain  TEXT    NOT NULL,
    source_seq INTEGER NOT NULL REFERENCES rust_round_source (seq),
    attempt    INTEGER NOT NULL CHECK (attempt >= 1),
    dropped_at TEXT    NOT NULL,
    PRIMARY KEY (toolchain, source_seq, attempt)
);

CREATE TRIGGER rust_round_is_append_only
BEFORE UPDATE ON rust_round
BEGIN
    SELECT RAISE(ABORT, 'rust_round is append-only: a round is never rewritten');
END;

CREATE TRIGGER rust_round_is_kept
BEFORE DELETE ON rust_round
BEGIN
    SELECT RAISE(ABORT, 'rust_round is append-only: a round is kept for good');
END;

CREATE TRIGGER rust_round_source_is_append_only
BEFORE UPDATE ON rust_round_source
BEGIN
    SELECT RAISE(ABORT, 'rust_round_source is append-only: a program is never rewritten');
END;

CREATE TRIGGER rust_round_source_is_kept
BEFORE DELETE ON rust_round_source
BEGIN
    SELECT RAISE(ABORT, 'rust_round_source is append-only: a program is kept for good');
END;

CREATE TRIGGER rust_round_run_is_append_only
BEFORE UPDATE ON rust_round_run
BEGIN
    SELECT RAISE(ABORT, 'rust_round_run is append-only: a run is never rewritten');
END;

CREATE TRIGGER rust_round_run_is_kept
BEFORE DELETE ON rust_round_run
BEGIN
    SELECT RAISE(ABORT, 'rust_round_run is append-only: a run is kept for good');
END;

CREATE TRIGGER rust_round_drop_is_append_only
BEFORE UPDATE ON rust_round_drop
BEGIN
    SELECT RAISE(ABORT, 'rust_round_drop is append-only');
END;

CREATE TRIGGER rust_round_drop_is_kept
BEFORE DELETE ON rust_round_drop
BEGIN
    SELECT RAISE(ABORT, 'rust_round_drop is append-only');
END;
