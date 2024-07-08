-- Add up migration script here
ALTER TABLE browser_tabs RENAME COLUMN "index" TO tab_index;