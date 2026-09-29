-- The trigger lives on `account`, so dropping the table would leave it
-- behind, failing every later account deletion. It goes first.
DROP TRIGGER learner_shelf_leaves_with_account;
DROP TABLE learner_shelf;
