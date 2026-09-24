-- #273 (CAT5): "new material exists" becomes a fact the server notices, and
-- `StudySignal::CurriculumUpdated` gets its first producer — without fanning
-- it out to anyone.
--
-- The catalogue arrives by migration, not by a write route (#271), so there is
-- no request to hang "this was just published" on. Instead each waker pass
-- compares `activities` against this log: an `(id, version)` pair it has not
-- recorded is a publication -- a new row *or* a version bump, both of which are
-- new material to someone who has played the old one. #277 (CUR4) appends
-- lesson content to the same log with `source = 'curriculum'`.
--
-- FAN-OUT-ON-READ, NOT ON WRITE. Publishing is one append here, whatever the
-- number of subjects. The log's `MAX(id)` is the **epoch**; each subject
-- carries one watermark, `engagement_gate.curriculum_epoch` (below), the
-- newest epoch already folded into their charge. The waker treats a subject
-- behind the epoch as due and catches them up -- one drain, watermark advanced
-- in the same transaction. Nothing records (publication, subject) pairs: state
-- is O(1) per publication and O(1) per subject, never their product. See
-- `docs/study-nudge.md`, "Scaling invariants".
--
-- No REFERENCES, matching every other table in this schema.
CREATE TABLE curriculum_publication (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,   -- the epoch
    source         TEXT    NOT NULL,   -- 'activity' (#273); 'curriculum' is #277's
    curriculum_id  TEXT    NOT NULL,   -- the activity id; the signal's `curriculum_id`
    version        INTEGER NOT NULL,
    detected_at    TEXT    NOT NULL,   -- ISO-8601 UTC

    UNIQUE (source, curriculum_id, version)
);

-- THE BASELINE. Everything already in the catalogue when this lands is what the
-- app has been serving all along -- not new material to anyone.
INSERT INTO curriculum_publication (source, curriculum_id, version, detected_at)
SELECT 'activity', id, version, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
FROM activities;

-- THE WATERMARK. Every gate row that exists today starts at the baseline epoch,
-- so deploying this drains nobody. A gate row created later is stamped with the
-- epoch current at its creation (`EngagementRepository`'s two inserts), so a
-- subject who arrives after a publication is never behind it.
ALTER TABLE engagement_gate ADD COLUMN curriculum_epoch INTEGER NOT NULL DEFAULT 0;
UPDATE engagement_gate SET curriculum_epoch = (SELECT COALESCE(MAX(id), 0) FROM curriculum_publication);

-- The second half of the waker's one query: `eligible_at <= now OR
-- curriculum_epoch < epoch`, answered from this index and
-- `idx_engagement_gate_eligible_at` together.
CREATE INDEX idx_engagement_gate_curriculum_epoch ON engagement_gate(curriculum_epoch);
