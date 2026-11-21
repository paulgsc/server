-- Reverses 20261121000200_hear_a_take_again.up.sql.
--
-- **It refuses rather than discards:** a `hear_again` act is the owner's
-- record, so once one is held this fails on the CHECK below and the
-- migration's transaction rolls back, leaving the table as it was.
CREATE TEMP TABLE voice_note_attention_hears_none_again (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO voice_note_attention_hears_none_again (n) SELECT COUNT(*) FROM voice_note_attention WHERE act = 'hear_again';
DROP TABLE voice_note_attention_hears_none_again;

CREATE TABLE voice_note_attention_prev (
    subject_id TEXT    NOT NULL,
    note_id    TEXT    NOT NULL,
    n          INTEGER NOT NULL CHECK (n >= 1),
    act        TEXT    NOT NULL CHECK (act IN ('kept', 'record_again', 'deleted')),
    model      TEXT    NOT NULL,
    attempt    INTEGER NOT NULL CHECK (attempt >= 1),
    rule       TEXT    NOT NULL,
    acted_at   TEXT    NOT NULL,
    PRIMARY KEY (subject_id, note_id, n)
);

INSERT INTO voice_note_attention_prev (subject_id, note_id, n, act, model, attempt, rule, acted_at)
    SELECT subject_id, note_id, n, act, model, attempt, rule, acted_at FROM voice_note_attention;

DROP TRIGGER voice_note_attention_leaves_with_account;
DROP TABLE voice_note_attention;
ALTER TABLE voice_note_attention_prev RENAME TO voice_note_attention;

CREATE TRIGGER voice_note_attention_is_append_only
BEFORE UPDATE ON voice_note_attention
BEGIN
    SELECT RAISE(ABORT, 'voice_note_attention is append-only: a later act is a new row');
END;

CREATE TRIGGER voice_note_attention_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM voice_note_attention WHERE subject_id = OLD.subject_id;
END;
