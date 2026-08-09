-- Add down migration script here
DROP INDEX IF EXISTS idx_mood_events_index_pos;

DROP INDEX IF EXISTS idx_mood_events_category;

DROP INDEX IF EXISTS idx_mood_events_team;

DROP INDEX IF EXISTS idx_mood_events_week;

DROP TABLE IF EXISTS mood_events;
