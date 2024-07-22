--- Add up migration script here
-- comment it out if no tansaction
-- ROLLBACK;

ALTER TABLE browser_tabs ADD COLUMN new_last_accessed INTEGER NOT NULL DEFAULT 0;

ALTER TABLE browser_tabs DROP COLUMN last_accessed;

ALTER TABLE browser_tabs RENAME COLUMN new_last_accessed TO last_accessed;

