-- Add up migration script here
-- comment it out if no tansaction
-- ROLLBACK;

ALTER TABLE browser_tabs ADD COLUMN new_tab_index INTEGER NOT NULL DEFAULT 0;
ALTER TABLE browser_tabs ADD COLUMN new_window_id INTEGER NOT NULL DEFAULT 0;
ALTER TABLE browser_tabs ADD COLUMN new_group_id INTEGER NOT NULL DEFAULT 0;

UPDATE browser_tabs SET new_tab_index = tab_index;
UPDATE browser_tabs SET new_window_id = window_id;
UPDATE browser_tabs SET new_group_id = group_id;

ALTER TABLE browser_tabs DROP COLUMN tab_index;
ALTER TABLE browser_tabs DROP COLUMN window_id;
ALTER TABLE browser_tabs DROP COLUMN group_id;


ALTER TABLE browser_tabs RENAME COLUMN new_tab_index TO tab_index;
ALTER TABLE browser_tabs RENAME COLUMN new_window_id TO window_id;
ALTER TABLE browser_tabs RENAME COLUMN new_group_id TO group_id;
