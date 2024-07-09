-- Add up migration script here
ALTER TABLE browser_tabs RENAME COLUMN reason TO muted_reason;
ALTER TABLE browser_tabs RENAME COLUMN extension_id TO muted_extension_id;