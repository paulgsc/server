-- A paired phone's person can now choose whether voice notes go home
-- (paulgsc/some-ui#2038, VOICE5), so `sharing_choice` takes a fifth scope,
-- `voice`, and home refuses `PUT /voice-notes/:id` from a pairing that
-- stopped it, as it refuses aph's and the Jobs record's writes
-- (docs/identity.md, invariant 17).
--
-- SQLite cannot alter a CHECK, so this rebuilds the table the way
-- `20260906000900_add_origin_to_sessions.up.sql` rebuilt `sessions`: a fresh
-- `sharing_choice_new` with the wider CHECK, every row copied across
-- unchanged, the old table dropped and the new one renamed in. The table is
-- append-only and its rows are the record of what was agreed to and when, so
-- the copy takes every row and changes none; the columns, their order and
-- the primary key are the original's, and only the CHECK on `scope` widens.
--
-- Columns are as `20261019000100_create_sharing_choice.up.sql` describes
-- them; `scope` is now `sessions`, `shelf`, `aph`, `jobs` or `voice`.
--
-- Both triggers are the original's, recreated word for word. The
-- append-only one lives on `sharing_choice` and goes with the dropped table.
-- The account one lives on `account` and names `sharing_choice` in its body,
-- so it is dropped first, before the table it names is gone, and recreated
-- once the rebuilt table holds the name again. Nothing else (no index, no
-- other trigger, no later migration) was created on this table.
DROP TRIGGER sharing_choice_leaves_with_account;

CREATE TABLE sharing_choice_new (
    subject_id TEXT    NOT NULL,
    pairing_id TEXT    NOT NULL,
    seq        INTEGER NOT NULL,
    scope      TEXT    NOT NULL CHECK (scope IN ('sessions', 'shelf', 'aph', 'jobs', 'voice')),
    kept       INTEGER NOT NULL CHECK (kept IN (0, 1)),
    at         TEXT    NOT NULL,
    PRIMARY KEY (subject_id, pairing_id, seq)
);

INSERT INTO sharing_choice_new (subject_id, pairing_id, seq, scope, kept, at)
SELECT subject_id, pairing_id, seq, scope, kept, at FROM sharing_choice;

DROP TABLE sharing_choice;
ALTER TABLE sharing_choice_new RENAME TO sharing_choice;

CREATE TRIGGER sharing_choice_is_append_only
BEFORE UPDATE ON sharing_choice
BEGIN
    SELECT RAISE(ABORT, 'sharing_choice is append-only: write a new change');
END;
-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `jobs_event` does.
CREATE TRIGGER sharing_choice_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM sharing_choice WHERE subject_id = OLD.subject_id;
END;
