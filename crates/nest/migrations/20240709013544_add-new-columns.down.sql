-- -- Add down migration script here
ALTER TABLE browser_tabs DROP COLUMN muted;
ALTER TABLE browser_tabs ADD COLUMN reason;
ALTER TABLE browser_tabs ADD COLUMN extension_id;
ALTER TABLE browser_tabs ADD COLUMN muted_info TEXT;