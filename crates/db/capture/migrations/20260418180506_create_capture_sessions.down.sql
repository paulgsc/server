-- Add down migration script here
-- migrations/0001_capture_sessions.down.sql

DROP INDEX IF EXISTS idx_capture_sessions_captured_at;
DROP INDEX IF EXISTS idx_capture_sessions_session_id;

DROP TABLE IF EXISTS capture_sessions;
