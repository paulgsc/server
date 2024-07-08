-- Add down migration script here
ALTER TABLE browser_tabs RENAME COLUMN tab_index TO "index";