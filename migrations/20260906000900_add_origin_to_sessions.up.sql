-- #283 (RCM6): a session must be able to say whether a person composed it or
-- the waker proposed it, because `Momentum` reads the answer and gets it
-- backwards if it guesses. See `SessionOrigin::parse` (`session_repo::model`)
-- for the "unrecognised value is refused, not defaulted" argument -- the
-- same one already made for `SessionStatus::parse`.
--
-- `origin TEXT NOT NULL` cannot be added via `ALTER TABLE ... ADD COLUMN`
-- without a default, and a lingering default is exactly what the acceptance
-- criteria forbid -- the same footgun
-- `20260816000500_add_subject_to_sessions.up.sql` already reasoned through
-- for `subject_id`. So this rebuilds the table the same way that migration
-- did: a fresh `sessions_new` declares `origin` `NOT NULL` with no default,
-- an `INSERT ... SELECT` backfills `'user'` for every existing row -- the
-- only possible answer, since every row that exists today was written before
-- the waker (`#279`) ever proposed anything -- and the rebuilt table is
-- swapped in.
CREATE TABLE sessions_new (
    id                TEXT    PRIMARY KEY,
    subject_id        TEXT    NOT NULL,
    name              TEXT    NOT NULL,
    status            TEXT    NOT NULL,
    origin            TEXT    NOT NULL,      -- user | system; see SessionOrigin::parse
    layout_mode       TEXT    NOT NULL,
    total_duration_ms INTEGER NOT NULL,

    created_at        TEXT    NOT NULL,
    updated_at        TEXT    NOT NULL,
    started_at        TEXT,
    completed_at      TEXT,
    final_elapsed_ms  INTEGER,

    activities        TEXT    NOT NULL,
    scenes            TEXT    NOT NULL,
    layout            TEXT
);

INSERT INTO sessions_new (
    id, subject_id, name, status, origin, layout_mode, total_duration_ms,
    created_at, updated_at, started_at, completed_at, final_elapsed_ms,
    activities, scenes, layout
)
SELECT
    id, subject_id, name, status, 'user', layout_mode, total_duration_ms,
    created_at, updated_at, started_at, completed_at, final_elapsed_ms,
    activities, scenes, layout
FROM sessions;

DROP TABLE sessions;
ALTER TABLE sessions_new RENAME TO sessions;

-- Unchanged from `20260816000500_add_subject_to_sessions.up.sql` -- neither
-- index reasons about `origin`, and nothing reads sessions filtered or
-- ordered by it yet.
CREATE INDEX idx_sessions_started_at ON sessions(started_at);
CREATE INDEX idx_sessions_completed_at ON sessions(completed_at);
CREATE INDEX idx_sessions_status ON sessions(subject_id, status, updated_at DESC);
