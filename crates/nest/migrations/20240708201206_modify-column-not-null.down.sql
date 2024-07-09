-- Add down migration script here


ALTER TABLE browser_tabs ADD COLUMN new_tab_index INTEGER;
ALTER TABLE browser_tabs ADD COLUMN new_window_id INTEGER;
ALTER TABLE browser_tabs  ADD COLUMN new_group_id INTEGER;

UPDATE browser_tabs SET new_tab_index = tab_index;
UPDATE browser_tabs SET new_window_id = window_id;
UPDATE browser_tabs SET new_group_id = group_id;

ALTER TABLE browser_tabs DROP COLUMN tab_index;
ALTER TABLE browser_tabs DROP COLUMN window_id;
ALTER TABLE browser_tabs DROP COLUMN group_id;


ALTER TABLE browser_tabs RENAME COLUMN new_tab_index TO tab_index;
ALTER TABLE browser_tabs RENAME COLUMN new_window_id TO window_id;
ALTER TABLE browser_tabs RENAME COLUMN new_group_id TO group_id;
