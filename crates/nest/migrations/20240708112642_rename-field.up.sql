-- Add up migration script here
ALTER TABLE browser_tabs RENAME COLUMN fav_icon_url TO favicon_url;