-- Add down migration script here
ALTER TABLE browser_tabs RENAME COLUMN favicon_url TO fav_icon_url;