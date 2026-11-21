-- The owner asks home to hear a take again (paulgsc/some-ui#3339, the owner's
-- scope, 2026-11-21: "it's what we have, except the retry"): a fourth act in
-- `voice_note_attention`'s closed set, `hear_again`, beside `kept`,
-- `record_again` and `deleted`.
--
-- A `hear_again` act answers a note's newest verdict, whatever it is, a heard
-- one included, names it by its key (model, attempt, rule) as every act does,
-- and says the key the note is heard again under, `hears_under`: the note's
-- key with `+again<k>`, `k` the requests made under that key
-- (`voice_repo::transcript::again`). A fresh transcription, the way the
-- interview's verbatim prompt is a key of its own: a model ends a note once
-- under a key (`voice_transcript_one_ending`), so nothing is rewritten and the
-- old ending, words and verdict stay beside the new. Asked again only once the
-- policy's snooze has passed (`voice_repo::attention::HEAR_AGAIN_SNOOZE`).
--
-- `voice_note_superseding` is the one place a reader learns what a request
-- superseded: each note's newest request, while home would still hear the
-- note (its recording held, and not set aside but for a Jobs note). Every
-- read that chooses a note's ending or final verdict skips a row whose model
-- is not its request's `hears_under`, whatever model wrote it; a mark set
-- aside or a recording let go ends the supersession, and the old rows read
-- again.
--
-- SQLite cannot change a CHECK in place or add a checked column, so the table
-- is rebuilt with the same rows, key, triggers and append-only rule, the act's
-- CHECK widened by the one name and `hears_under` added, NULL for every other
-- act. Nothing else changes.
CREATE TABLE voice_note_attention_next (
    subject_id TEXT    NOT NULL,
    note_id    TEXT    NOT NULL,
    n          INTEGER NOT NULL CHECK (n >= 1),
    act        TEXT    NOT NULL CHECK (act IN ('kept', 'record_again', 'deleted', 'hear_again')),
    model      TEXT    NOT NULL,
    attempt    INTEGER NOT NULL CHECK (attempt >= 1),
    rule       TEXT    NOT NULL,
    acted_at   TEXT    NOT NULL,
    hears_under TEXT,
    PRIMARY KEY (subject_id, note_id, n),
    CHECK ((act = 'hear_again') = (hears_under IS NOT NULL))
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

-- Each note's newest request to hear it again, while home would still hear
-- the note: what every reader that chooses an ending or a verdict skips the
-- note's other keys for (`voice_repo::transcript::SUPERSEDED`).
CREATE VIEW voice_note_superseding AS
SELECT h.subject_id, h.note_id, h.hears_under
FROM voice_note_attention h
JOIN voice_note n ON n.subject_id = h.subject_id AND n.note_id = h.note_id
WHERE h.act = 'hear_again'
  AND h.n = (SELECT MAX(l.n) FROM voice_note_attention l
             WHERE l.subject_id = h.subject_id AND l.note_id = h.note_id AND l.act = 'hear_again')
  AND EXISTS (SELECT 1 FROM voice_note_audio a WHERE a.subject_id = h.subject_id AND a.note_id = h.note_id)
  AND (n.of_kind = 'jobs' OR NOT EXISTS (SELECT 1 FROM voice_note_set_aside s
                                         WHERE s.subject_id = h.subject_id AND s.note_id = h.note_id));
