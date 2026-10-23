-- The account triggers live on `account`, so dropping a table would leave
-- them behind, failing every later account deletion. They go first.
DROP TRIGGER mailbox_handed_leaves_with_account;
DROP TRIGGER mailbox_outgoing_leaves_with_account;
DROP TRIGGER mailbox_item_leaves_with_account;
DROP TRIGGER mailbox_handed_is_append_only;
DROP TRIGGER mailbox_outgoing_is_append_only;
DROP TRIGGER mailbox_item_is_append_only;
DROP INDEX mailbox_handed_file;
DROP INDEX mailbox_item_seq;
DROP TABLE mailbox_handed;
DROP TABLE mailbox_outgoing;
DROP TABLE mailbox_item;
