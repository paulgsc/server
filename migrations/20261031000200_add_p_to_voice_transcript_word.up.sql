-- How sure the model was of each heard word (paulgsc/server#577, STUDIO
-- ST22): whisper.cpp's probability for the word, the lowest of its tokens',
-- so one unsure piece makes the word unsure. A take check and the owner's
-- review of a take can then tell a solid word from a guess, and captions
-- need not present every word as certain.
--
-- Nullable: a word heard before this column existed has none, and is never
-- given one (the table is append-only; a better model's hearing adds rows of
-- its own). Where it is set it is 0 to 1 (docs/identity.md invariant 23).
ALTER TABLE voice_transcript_word ADD COLUMN p REAL CHECK (p IS NULL OR (p >= 0 AND p <= 1));
