-- #273 (CAT5): "new material exists" becomes a fact the server notices, and
-- `StudySignal::CurriculumUpdated` gets its first producer.
--
-- The catalogue arrives by migration, not by a write route (#271), so there is
-- no request to hang "this was just published" on. Instead each waker pass
-- compares `activities` against this table: an `(id, version)` pair it has not
-- recorded is a publication -- a new row *or* a version bump, both of which are
-- new material to someone who has played the old one. #277 (CUR4) reuses the
-- same two tables for lesson content with `source = 'curriculum'`, rather than
-- growing a second mechanism.
--
--   curriculum_publication -- one row per (source, curriculum_id, version) ever
--                             seen. `fanned_out_at` NULL means its audience is
--                             still being worked through; set once the fan-out
--                             has reached everyone it applies to.
--   curriculum_delivery    -- one row per (publication, subject) the signal has
--                             been applied to. The PRIMARY KEY is the
--                             idempotency guarantee: the row is claimed *before*
--                             the signal is folded, so re-running a publish --
--                             or a pass that crashed half-way through one --
--                             can never drain a subject twice. A crash between
--                             claim and fold costs that one subject that one
--                             drain, which is the right way round (the same
--                             reasoning as the waker's claim-before-send).
--
-- No REFERENCES, matching every other table in this schema.
CREATE TABLE curriculum_publication (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    source         TEXT    NOT NULL,   -- 'activity' (#273); 'curriculum' is #277's
    curriculum_id  TEXT    NOT NULL,   -- the activity id; the signal's `curriculum_id`
    version        INTEGER NOT NULL,
    detected_at    TEXT    NOT NULL,   -- ISO-8601 UTC
    fanned_out_at  TEXT,               -- ISO-8601 UTC; NULL while in progress
    -- The last subject_id this publication's fan-out has applied to, in
    -- subject order. The next pass's audience read seeks past it rather than
    -- rescanning everyone already reached; NULL before the first batch.
    cursor_subject TEXT,

    UNIQUE (source, curriculum_id, version)
);

-- The waker's "what is still being fanned out" read.
CREATE INDEX idx_curriculum_publication_pending ON curriculum_publication(fanned_out_at, id);

-- The audience read for a catalogue publication (subjects who have started a
-- session, in subject order, past the cursor) seeks this partial index
-- directly: it holds only started sessions, so a pass examines the sessions of
-- the subjects it returns, not the whole table's history.
CREATE INDEX idx_sessions_started_subject ON sessions(subject_id) WHERE started_at IS NOT NULL;

CREATE TABLE curriculum_delivery (
    publication_id INTEGER NOT NULL,
    subject_id     TEXT    NOT NULL,
    applied_at     TEXT    NOT NULL,   -- ISO-8601 UTC

    PRIMARY KEY (publication_id, subject_id)
);

-- THE BASELINE. Everything already in the catalogue when this lands is what the
-- app has been serving all along -- not new material to anyone. Recorded as
-- already fanned out, so the first pass after deploy announces nothing, rather
-- than draining every subject's freshness by 35 on the day this ships.
INSERT INTO curriculum_publication (source, curriculum_id, version, detected_at, fanned_out_at)
SELECT 'activity', id, version, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
FROM activities;
