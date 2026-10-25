-- The account trigger lives on `account`, so dropping the tables would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER voice_note_leaves_with_account;
DROP TRIGGER voice_note_audio_is_append_only;
DROP TRIGGER voice_note_is_append_only;
DROP TABLE voice_note_audio;
DROP TABLE voice_note;
