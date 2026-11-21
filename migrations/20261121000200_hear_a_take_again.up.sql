-- The owner asks home to hear a take again (paulgsc/some-ui#3339, the owner's
-- scope, 2026-11-21: "it's what we have, except the retry"): a fourth act in
-- `voice_note_attention`'s closed set, `hear_again`, beside `kept`,
-- `record_again` and `deleted`.
--
-- A `hear_again` act answers a note's newest verdict, which must be `failed`
-- (no attempt ended it), and names it by its key (model, attempt, rule), as
-- every act does. The transcriber then counts a fresh set of its attempts on
-- that note: only those numbered after the act's `attempt`, by the act's
-- `model` (`voice_repo::transcript::waiting`). No attempt is rewritten, and
-- the old verdict stays. Asked again only once the policy's snooze has passed
-- (`voice_repo::attention::HEAR_AGAIN_SNOOZE`).
--
-- SQLite cannot change a CHECK in place, so the table is rebuilt with the
-- same columns, rows, key, triggers and append-only rule, the act's CHECK
-- widened by the one name. Nothing else changes.
CREATE TABLE voice_note_attention_next (
    subject_id TEXT    NOT NULL,
    note_id    TEXT    NOT NULL,
    n          INTEGER NOT NULL CHECK (n >= 1),
    act        TEXT    NOT NULL CHECK (act IN ('kept', 'record_again', 'deleted', 'hear_again')),
    model      TEXT    NOT NULL,
    attempt    INTEGER NOT NULL CHECK (attempt >= 1),
    rule       TEXT    NOT NULL,
    acted_at   TEXT    NOT NULL,
    PRIMARY KEY (subject_id, note_id, n)
);

INSERT INTO voice_note_attention_next (subject_id, note_id, n, act, model, attempt, rule, acted_at)
    SELECT subject_id, note_id, n, act, model, attempt, rule, acted_at FROM voice_note_attention;

-- Its own triggers go with it; the account's, which names it, goes first, so
-- the rename finds no trigger naming a table that is not there.
DROP TRIGGER voice_note_attention_leaves_with_account;
DROP TABLE voice_note_attention;
ALTER TABLE voice_note_attention_next RENAME TO voice_note_attention;

CREATE TRIGGER voice_note_attention_is_append_only
BEFORE UPDATE ON voice_note_attention
BEGIN
    SELECT RAISE(ABORT, 'voice_note_attention is append-only: a later act is a new row');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `voice_transcript_verdict` does.
CREATE TRIGGER voice_note_attention_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM voice_note_attention WHERE subject_id = OLD.subject_id;
END;
