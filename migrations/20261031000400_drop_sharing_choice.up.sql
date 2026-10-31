-- Home no longer keeps what a paired phone chose to send (paulgsc/some-ui#2182,
-- HOME-FIRST HF8; the owner's decision D3 on paulgsc/some-ui#2174: no
-- per-domain switch, and no consent log told to home or enforced by home).
-- Home is the owner's own store, as private as the phone, so a paired phone
-- sends everything it keeps, and nothing here refuses a write as stopped.
-- `sharing_choice` was what home kept of those choices, read only to refuse
-- such writes (docs/identity.md, invariant 17, now retired), so it goes.
--
-- Its rows go with it. They were the times each choice began and ended
-- under a pairing, and nothing the owner studied or recorded: what the
-- phone sent under them stays in its own tables.
--
-- The account trigger lives on `account` and names `sharing_choice` in its
-- body, so dropping the table alone would leave it behind, failing every
-- later account deletion. It goes first, as in
-- 20261019000100_create_sharing_choice.down.sql; the append-only trigger
-- lives on the table, and is dropped by name all the same.
DROP TRIGGER sharing_choice_leaves_with_account;
DROP TRIGGER sharing_choice_is_append_only;
DROP TABLE sharing_choice;
