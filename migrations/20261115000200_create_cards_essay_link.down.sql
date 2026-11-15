-- Reverses 20261115000200_create_cards_essay_link.up.sql, refusing rather
-- than discarding once the table holds a row, as pd_video's down migration
-- does: a link is the owner's record.
CREATE TEMP TABLE cards_essay_link_is_empty (n INTEGER NOT NULL CHECK (n = 0));
INSERT INTO cards_essay_link_is_empty (n) SELECT COUNT(*) FROM cards_essay_link;
DROP TABLE cards_essay_link_is_empty;

DROP TRIGGER cards_essay_link_no_delete;
DROP TRIGGER cards_essay_link_no_update;
DROP TABLE cards_essay_link;
