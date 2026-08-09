-- Add down migration script here
-- Remove indices first to clean up the schema explicitly
DROP INDEX IF EXISTS idx_tabs_domain;
DROP INDEX IF EXISTS idx_tabs_last_seen_at;

-- Remove the primary table
DROP TABLE IF EXISTS tabs;
