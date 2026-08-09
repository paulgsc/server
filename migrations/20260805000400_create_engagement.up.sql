-- The durable half of the intervention engine.
--
-- Two tables and one index, and the index is the whole scheduler.

-- One row per (subject, engagement class). Levels are stored UNDECAYED,
-- alongside the instant they are as of; decay is a closed-form function of
-- that stamp and is computed on read. Nothing ticks these rows, which is what
-- makes an idle subject cost zero.
CREATE TABLE engagement_charge (
    subject_id   TEXT    NOT NULL,
    -- `Enumerable::discriminant`, not the dense index: this is the durable
    -- numbering and is append-only. A discriminant this build does not
    -- recognise is quarantined on read rather than reinterpreted.
    class        INTEGER NOT NULL,
    level        REAL    NOT NULL,
    as_of        TEXT    NOT NULL,   -- ISO-8601 UTC

    PRIMARY KEY (subject_id, class)
);

-- Per-subject gate state. `eligible_at` is the solved instant at which this
-- subject's weighted charge falls to the threshold — computed once per signal,
-- never polled toward.
CREATE TABLE engagement_gate (
    subject_id          TEXT PRIMARY KEY,

    -- THE SCHEDULER. The waker is `WHERE eligible_at <= now`, which discovers
    -- subjects the arithmetic already marked. It decides nothing.
    eligible_at         TEXT NOT NULL,

    last_intervened_at  TEXT,        -- ISO-8601 UTC; refractory is measured from here
    last_action         TEXT,        -- StudyAction::kind, for diagnosis
    intervention_count  INTEGER NOT NULL DEFAULT 0
);

-- The one query the waker runs. Without this index it is a table scan per
-- pass, which is the difference between "lazy" and "a cron with extra steps".
CREATE INDEX idx_engagement_gate_eligible_at ON engagement_gate(eligible_at);

-- What actually went out. Separate from the gate because the gate is current
-- state and this is history: it answers "why did I get that" weeks later, and
-- it is what makes a redelivery detectable rather than invisible.
CREATE TABLE intervention_log (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_id   TEXT    NOT NULL,
    action_kind  TEXT    NOT NULL,
    action       TEXT    NOT NULL,   -- JSON; the full StudyAction
    decided_at   TEXT    NOT NULL,   -- ISO-8601 UTC
    -- Set once delivery was attempted. NULL means claimed-but-not-yet-sent,
    -- which is the window a crash can leave behind and the reason the claim
    -- and the send are ordered this way rather than the other.
    actuated_at  TEXT
);

CREATE INDEX idx_intervention_log_subject ON intervention_log(subject_id, decided_at DESC);
