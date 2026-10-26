-- Reverses 20261026000300_add_korean_to_sharing_choice.up.sql: rebuilds
-- `sharing_choice` back to the five scopes
-- 20261025000300_add_voice_to_sharing_choice.up.sql allowed, the same way.
--
-- **It refuses rather than discards,** as that migration's down does: every
-- row is copied, a `korean` one included, into a table whose CHECK refuses
-- `korean`, so once any pairing has made a Korean choice this fails on that
-- row and the migration's transaction rolls back, leaving the table as it
-- was.
--
-- The account trigger names this table, so it goes first and comes back last.
DROP TRIGGER sharing_choice_leaves_with_account;

CREATE TABLE sharing_choice_old (
    subject_id TEXT    NOT NULL,
    pairing_id TEXT    NOT NULL,
    seq        INTEGER NOT NULL,
    scope      TEXT    NOT NULL CHECK (scope IN ('sessions', 'shelf', 'aph', 'jobs', 'voice')),
    kept       INTEGER NOT NULL CHECK (kept IN (0, 1)),
    at         TEXT    NOT NULL,
    PRIMARY KEY (subject_id, pairing_id, seq)
);

INSERT INTO sharing_choice_old (subject_id, pairing_id, seq, scope, kept, at)
SELECT subject_id, pairing_id, seq, scope, kept, at FROM sharing_choice;

DROP TABLE sharing_choice;
ALTER TABLE sharing_choice_old RENAME TO sharing_choice;

CREATE TRIGGER sharing_choice_is_append_only
BEFORE UPDATE ON sharing_choice
BEGIN
    SELECT RAISE(ABORT, 'sharing_choice is append-only: write a new change');
END;
CREATE TRIGGER sharing_choice_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM sharing_choice WHERE subject_id = OLD.subject_id;
END;
