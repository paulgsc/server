-- Add down migration script here
ALTER TABLE browser_tabs RENAME COLUMN muted_reason TO reason;
ALTER TABLE browser_tabs RENAME COLUMN muted_extension_id TO extension_id;