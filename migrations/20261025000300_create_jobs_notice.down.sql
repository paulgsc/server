-- The account trigger lives on `account`, so dropping the tables would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER jobs_notice_leaves_with_account;
DROP TRIGGER jobs_notice_sent_is_append_only;
DROP TABLE jobs_notice_sent;
DROP TABLE jobs_notice_device;
