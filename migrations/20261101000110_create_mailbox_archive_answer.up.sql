-- The mailbox's answer each time home asked it to archive a written
-- reflection (paulgsc/some-ui#2178, HF3). Home stores a file's bytes in
-- `mailbox_archive` before it asks the mailbox to drop them (invariant 21),
-- so a row there says home holds the bytes, not that the mailbox let them
-- go: an ask that failed to reach the mailbox, or whose answer was lost, or
-- a restart between the two, leaves the file live there. Every other folder
-- is listed on each pass, so a file still live is found again; the
-- `reflection` folder is listed only while one is due by home's own tables
-- (`mailbox_repo::reflection_due`), so this table is what tells home an ask
-- is still owed. A reflection archived here with no answer is asked for
-- again, by its name and the bytes home holds, without listing the folder
-- (`mailbox_repo::unanswered_archives`). Any answer is final, as a hand-off's
-- is (`mailbox_handed`); only a failure to reach the mailbox asks again.
--
-- APPEND-ONLY, as the rest of home's copy is (invariant 19): no row is
-- updated (the trigger aborts an `UPDATE`), and nothing deletes but account
-- deletion.
--
-- Columns, one row each time home asked and heard an answer:
--
--   subject_id  -- whose file.
--   folder      -- the mailbox folder: `reflection`.
--   name        -- as in `mailbox_archive`.
--   answer      -- what the mailbox said: it dropped the bytes (`archived`),
--                  had already (`unchanged`), holds other bytes under the
--                  name (`mismatch`), refused (`refused`), or holds nothing
--                  under it (`gone`).
--   answered_at -- ISO-8601 UTC, when it answered.
CREATE TABLE mailbox_archive_answer (
    subject_id  TEXT NOT NULL,
    folder      TEXT NOT NULL,
    name        TEXT NOT NULL,
    answer      TEXT NOT NULL CHECK (answer IN ('archived', 'unchanged', 'mismatch', 'refused', 'gone')),
    answered_at TEXT NOT NULL
);

CREATE INDEX mailbox_archive_answer_file ON mailbox_archive_answer (subject_id, folder, name);

CREATE TRIGGER mailbox_archive_answer_is_append_only
BEFORE UPDATE ON mailbox_archive_answer
BEGIN
    SELECT RAISE(ABORT, 'mailbox_archive_answer is append-only: write a new answer');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `mailbox_archive` does.
CREATE TRIGGER mailbox_archive_answer_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM mailbox_archive_answer WHERE subject_id = OLD.subject_id;
END;
