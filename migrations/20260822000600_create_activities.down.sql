-- Reverses 20260822000600_create_activities.up.sql.
DROP INDEX IF EXISTS idx_activities_maturity;
DROP INDEX IF EXISTS idx_activities_published_at;
DROP INDEX IF EXISTS idx_activities_registry_key;
DROP TABLE IF EXISTS activities;
