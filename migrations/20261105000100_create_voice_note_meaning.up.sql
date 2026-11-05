-- The take library's meaning index (paulgsc/some-ui#2590, ST32;
-- paulgsc/server#675): one vector per Studio take and embedding model, so
-- the owner can find "the take where I talked about the brand" by what it
-- means, not only by its words. docs/take-library.md, "Where the meaning
-- index lives", records the design call; in short:
--
-- - **Derived.** A vector is made from the take's chosen words (the
--   hearing `voice_repo::transcript::heard` takes, the one the library
--   lists), by a static embedding model home holds and runs on its CPU
--   (`file_host::meaning`). It can always be made again from the
--   transcripts, so it is not a record: a take whose chosen words change
--   (a better transcript) has its vector replaced, and a better embedding
--   model's vectors go under a new model name beside the old.
-- - **Written where the words are,** in the transcriber's pass, in the
--   transaction that keeps a hearing, and for a take heard before the index
--   existed by the same pass's bounded catch-up (`voice_repo::meaning`).
-- - **Takes only.** A row is written only for a `studio` note the subject
--   holds, in the statement that writes it.
-- - **What was said, kept as such** (docs/identity.md invariant 35): a
--   vector is subject-scoped, never logged, never filed for the mailbox and
--   never answered by a route; the library's search answers takes, not
--   vectors. It holds nothing but the note's id, the model and the vector:
--   no ref, no text, no time.
-- - **It leaves with the take**: My data's delete of a take
--   (`voice_repo::delete`) removes its vectors in the same transaction, and
--   the account sweep and the trigger below remove them with the account
--   (invariants 9, 12 and 22).
--
-- Columns of `voice_note_meaning`:
--
--   subject_id -- whose take.
--   note_id    -- the take (`voice_note.note_id`), the phone's id.
--   model      -- the embedding model the vector was made by.
--   vector     -- 256 IEEE-754 single-precision floats, little-endian, unit
--                 length: 1,024 bytes.
CREATE TABLE voice_note_meaning (
    subject_id TEXT NOT NULL,
    note_id    TEXT NOT NULL,
    model      TEXT NOT NULL CHECK (length(model) BETWEEN 1 AND 100),
    vector     BLOB NOT NULL CHECK (typeof(vector) = 'blob' AND length(vector) = 1024),
    PRIMARY KEY (subject_id, note_id, model)
);

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `voice_note` does.
CREATE TRIGGER voice_note_meaning_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM voice_note_meaning WHERE subject_id = OLD.subject_id;
END;
