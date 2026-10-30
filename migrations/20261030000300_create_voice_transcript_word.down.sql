-- The account trigger lives on `account`, so dropping the table would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER voice_transcript_word_leaves_with_account;
DROP TRIGGER voice_transcript_word_is_append_only;
DROP TABLE voice_transcript_word;
