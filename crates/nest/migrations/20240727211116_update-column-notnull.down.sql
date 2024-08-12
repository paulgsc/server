-- comment it out if no tansaction
-- ROLLBACK;

ALTER TABLE browser_tabs ADD COLUMN new_muted BOOLEAN;

UPDATE browser_tabs SET new_muted = muted;

ALTER TABLE browser_tabs DROP COLUMN muted;

ALTER TABLE browser_tabs RENAME COLUMN new_muted TO muted;
