-- The account trigger lives on `account`, so dropping the table would leave
-- it behind, failing every later account deletion. It goes first.
DROP TRIGGER sharing_choice_leaves_with_account;
DROP TRIGGER sharing_choice_is_append_only;
DROP TABLE sharing_choice;
