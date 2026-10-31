-- The language each heard word was heard in (paulgsc/server#578, STUDIO
-- ST23): home now hears each piece of speech the VAD found in its own
-- language, so a Korean phrase inside an English take comes back in Hangul,
-- and captions and the owner's review can show which word is which.
--
-- Nullable: a word heard before this column existed has none (the table is
-- append-only). Where it is set it is Whisper's code for the language, two
-- or three lower-case letters, as `voice_transcript.language` holds it.
ALTER TABLE voice_transcript_word ADD COLUMN language TEXT CHECK (language IS NULL OR language GLOB '[a-z][a-z]' OR language GLOB '[a-z][a-z][a-z]');
