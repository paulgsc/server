-- Reverses 20261105000100_create_voice_note_meaning.up.sql.
--
-- The table is derived (every vector is made again from the transcripts),
-- so dropping it loses nothing the transcripts do not hold.
--
-- The account trigger lives on `account`, so dropping the table would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER voice_note_meaning_leaves_with_account;
DROP TABLE voice_note_meaning;
