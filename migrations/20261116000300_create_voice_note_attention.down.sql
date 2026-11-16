-- The account trigger lives on `account`, so dropping the table would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER voice_note_attention_leaves_with_account;
DROP TRIGGER voice_note_attention_is_append_only;
DROP TABLE voice_note_attention;
