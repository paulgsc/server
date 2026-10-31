-- Reverses 20261031000400_drop_sharing_choice.up.sql: recreates
-- `sharing_choice` empty, in the shape
-- 20261026000300_add_korean_to_sharing_choice.up.sql left it, so every
-- earlier migration's down still finds the table it rebuilds or drops.
--
-- The rows the up dropped are not restored; nothing held a copy of them.
--
-- The columns, their order, the primary key, the CHECKs (six scopes:
-- `sessions`, `shelf`, `aph`, `jobs`, `voice`, `korean`) and both triggers
-- are 20261026000300's, word for word.
CREATE TABLE sharing_choice (
    subject_id TEXT    NOT NULL,
    pairing_id TEXT    NOT NULL,
    seq        INTEGER NOT NULL,
    scope      TEXT    NOT NULL CHECK (scope IN ('sessions', 'shelf', 'aph', 'jobs', 'voice', 'korean')),
    kept       INTEGER NOT NULL CHECK (kept IN (0, 1)),
    at         TEXT    NOT NULL,
    PRIMARY KEY (subject_id, pairing_id, seq)
);

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
