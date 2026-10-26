-- A paired phone's person can choose whether Korean's record goes home
-- (paulgsc/some-ui#2100, KO-HOME), so `sharing_choice` takes a sixth scope,
-- `korean`, and home refuses Korean's writes from a pairing that stopped it,
-- as it refuses aph's, the Jobs record's and voice notes' (docs/identity.md,
-- invariant 17).
--
-- Rebuilt exactly as 20261025000300_add_voice_to_sharing_choice.up.sql
-- rebuilt it, for the same reason (SQLite cannot alter a CHECK): a fresh
-- `sharing_choice_new` with the wider CHECK, every row copied across
-- unchanged, the old table dropped and the new one renamed in. Only the
-- CHECK on `scope` widens; `scope` is now `sessions`, `shelf`, `aph`,
-- `jobs`, `voice` or `korean`.
--
-- Both triggers are recreated word for word. The account one names
-- `sharing_choice` in its body, so it is dropped first and recreated once
-- the rebuilt table holds the name again.
DROP TRIGGER sharing_choice_leaves_with_account;

CREATE TABLE sharing_choice_new (
    subject_id TEXT    NOT NULL,
    pairing_id TEXT    NOT NULL,
    seq        INTEGER NOT NULL,
    scope      TEXT    NOT NULL CHECK (scope IN ('sessions', 'shelf', 'aph', 'jobs', 'voice', 'korean')),
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
