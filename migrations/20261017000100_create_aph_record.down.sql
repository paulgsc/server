-- The account triggers live on `account`, so dropping the tables would leave
-- them behind, failing every later account deletion. They go first.
DROP TRIGGER aph_settings_leaves_with_account;
DROP TRIGGER aph_entry_leaves_with_account;
DROP TRIGGER aph_settings_is_append_only;
DROP TRIGGER aph_entry_is_append_only;
DROP TABLE aph_settings;
DROP INDEX aph_entry_slot;
DROP TABLE aph_entry;
