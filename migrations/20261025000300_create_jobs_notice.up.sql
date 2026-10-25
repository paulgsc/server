-- The Jobs arrival notice (paulgsc/server#470, RUN4): home tells the owner's
-- phone, through Firebase Cloud Messaging, when a morning run's record
-- arrives, so the owner need not open Jobs to learn whether it ran.
--
-- `jobs_notice_device` holds the phones that asked to be told: each one's FCM
-- registration token, which names that install of the app to Google (as a
-- push endpoint names a browser to its push service, docs/identity.md), and
-- is never logged (invariant 24). A phone that stops asking, or that FCM says
-- is gone, is deleted: the row is a standing request, not a record.
--
-- `jobs_notice_sent` is the record: one row per run record home noticed, so a
-- record notifies at most once, across restarts. It is APPEND-ONLY (the
-- trigger below aborts an UPDATE), and holds what home said and to how many
-- phones, never which.
--
-- Columns of `jobs_notice_device`:
--
--   subject_id    -- whose phone.
--   token         -- the phone's FCM registration token, as the app read it.
--   registered_at -- ISO-8601 UTC, when the phone asked.
--
-- Columns of `jobs_notice_sent`:
--
--   subject_id -- whose run.
--   run_name   -- the run record's name in the mailbox (`run-jobs-<stamp>.json`).
--   said       -- `picks` (the picks are at home), `trouble` (the run couldn't
--                 finish) or `skipped` (a re-run that added nothing, on a
--                 morning whose picks another record added: nothing sent).
--   devices    -- how many phones it was handed to FCM for.
--   at         -- ISO-8601 UTC, when.
CREATE TABLE jobs_notice_device (
    subject_id    TEXT NOT NULL,
    token         TEXT NOT NULL CHECK (length(token) BETWEEN 1 AND 4096),
    registered_at TEXT NOT NULL,
    PRIMARY KEY (subject_id, token)
);

CREATE TABLE jobs_notice_sent (
    subject_id TEXT    NOT NULL,
    run_name   TEXT    NOT NULL,
    said       TEXT    NOT NULL CHECK (said IN ('picks', 'trouble', 'skipped')),
    devices    INTEGER NOT NULL CHECK (devices >= 0),
    at         TEXT    NOT NULL,
    PRIMARY KEY (subject_id, run_name)
);

CREATE TRIGGER jobs_notice_sent_is_append_only
BEFORE UPDATE ON jobs_notice_sent
BEGIN
    SELECT RAISE(ABORT, 'jobs_notice_sent is append-only: what home said is never rewritten');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `voice_note` does.
CREATE TRIGGER jobs_notice_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM jobs_notice_device WHERE subject_id = OLD.subject_id;
    DELETE FROM jobs_notice_sent WHERE subject_id = OLD.subject_id;
END;
