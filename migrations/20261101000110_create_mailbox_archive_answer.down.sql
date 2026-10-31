-- The account trigger lives on `account`, so dropping the table would leave
-- it behind, failing every later account deletion. It goes first. Dropping
-- the answers loses no record: home asks again for each reflection it
-- archived, and the mailbox answers `unchanged` for one it already dropped.
DROP TRIGGER mailbox_archive_answer_leaves_with_account;
DROP TRIGGER mailbox_archive_answer_is_append_only;
DROP TABLE mailbox_archive_answer;
