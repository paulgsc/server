-- The account trigger lives on `account`, so dropping the table would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER voice_transcript_leaves_with_account;
DROP TRIGGER voice_transcript_is_append_only;
DROP INDEX voice_note_by_stored_at;
DROP INDEX voice_transcript_one_ending;
DROP TABLE voice_transcript;
