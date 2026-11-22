-- The owner asks home to hear a take again (paulgsc/some-ui#3339, the owner's
-- scope, 2026-11-21: "it's what we have, except the retry"): a fourth act in
-- `voice_note_attention`'s closed set, `hear_again`, beside `kept`,
-- `record_again` and `deleted`.
--
-- A `hear_again` act answers a note's newest verdict, whatever it is, a heard
-- one included, names it by its key (model, attempt, rule) as every act does,
-- and says two keys: `asked_under`, the key home heard the note under when it
-- was asked (the model's, or its verbatim key for an interview take), and
-- `hears_under`, the fresh key home hears it again under: `asked_under` with
-- `+again<k>`, `k` the requests made under that key
-- (`voice_repo::transcript::again`). A fresh transcription, the way the
-- interview's verbatim prompt is a key of its own: a model ends a note once
-- under a key (`voice_transcript_one_ending`), so nothing is rewritten and the
-- old ending, words and verdict stay beside the new. The queue reads a note's
-- newest request asked under the key it hears the note under now; one asked
-- under another model's key is moot. Asked again only once the policy's
-- snooze has passed (`voice_repo::attention::HEAR_AGAIN_SNOOZE`).
--
-- `voice_note_hear_again_supersedes` lists, per request, the keys that had
-- made something of the note when it was asked: what that request may
-- supersede, and nothing else, so a key that comes later (a new model) is
-- never superseded by an older request. `voice_note_superseded` is the one
-- place a reader learns what is superseded: a request's listed keys, once its
-- fresh key has ended the note (heard, or found silent). Until then the old
-- words stay what every reader chooses, and the phone marks the take "being
-- heard again" (the owner, 2026-11-22, answer (b)); if every fresh attempt
-- fails, nothing is superseded and the old words stay (answer (a)). It reads
-- only the acts, the list and the transcripts, all append-only, so it never
-- lapses: a recording let go or a mark set aside afterwards changes nothing
-- of what was heard. No clock is read.
--
-- SQLite cannot change a CHECK in place or add a checked column, so the table
-- is rebuilt with the same rows, key, triggers and append-only rule, the act's
-- CHECK widened by the one name and the two keys added, NULL for every other
-- act. Nothing else changes.
CREATE TABLE voice_note_attention_next (
    subject_id  TEXT    NOT NULL,
    note_id     TEXT    NOT NULL,
    n           INTEGER NOT NULL CHECK (n >= 1),
    act         TEXT    NOT NULL CHECK (act IN ('kept', 'record_again', 'deleted', 'hear_again')),
    model       TEXT    NOT NULL,
    attempt     INTEGER NOT NULL CHECK (attempt >= 1),
    rule        TEXT    NOT NULL,
    acted_at    TEXT    NOT NULL,
    asked_under TEXT,
    hears_under TEXT,
    PRIMARY KEY (subject_id, note_id, n),
    CHECK ((act = 'hear_again') = (asked_under IS NOT NULL)),
    CHECK ((act = 'hear_again') = (hears_under IS NOT NULL)),
    CHECK (hears_under IS NULL OR hears_under <> asked_under)
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

-- The keys each request may supersede: every key that had made something of
-- the note (a transcript row) when it was asked, written in the request's own
-- transaction. Keyed for the readers' lookup by (subject, note, model).
CREATE TABLE voice_note_hear_again_supersedes (
    subject_id TEXT    NOT NULL,
    note_id    TEXT    NOT NULL,
    model      TEXT    NOT NULL,
    n          INTEGER NOT NULL CHECK (n >= 1),
    PRIMARY KEY (subject_id, note_id, model, n)
);

CREATE TRIGGER voice_note_hear_again_supersedes_is_append_only
BEFORE UPDATE ON voice_note_hear_again_supersedes
BEGIN
    SELECT RAISE(ABORT, 'voice_note_hear_again_supersedes is append-only: a later request is new rows');
END;

CREATE TRIGGER voice_note_hear_again_supersedes_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM voice_note_hear_again_supersedes WHERE subject_id = OLD.subject_id;
END;

-- What every reader that chooses an ending, words or a verdict skips
-- (`voice_repo::transcript::superseded`): a request's listed keys, once its
-- fresh key has ended the note.
CREATE VIEW voice_note_superseded AS
SELECT k.subject_id, k.note_id, k.model
FROM voice_note_hear_again_supersedes k
JOIN voice_note_attention r ON r.subject_id = k.subject_id AND r.note_id = k.note_id AND r.n = k.n
WHERE r.act = 'hear_again'
  AND EXISTS (SELECT 1 FROM voice_transcript t
              WHERE t.subject_id = r.subject_id AND t.note_id = r.note_id AND t.model = r.hears_under
                AND t.outcome IN ('heard', 'silent'));
