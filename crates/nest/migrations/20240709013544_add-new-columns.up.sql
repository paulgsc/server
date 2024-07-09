-- Add up migration script here
ALTER TABLE browser_tabs ADD COLUMN muted BOOLEAN;
ALTER TABLE browser_tabs ADD COLUMN reason TEXT;
ALTER TABLE browser_tabs ADD COLUMN extension_id TEXT;
ALTER TABLE browser_tabs DROP COLUMN muted_info;