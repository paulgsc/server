-- Add up migration script here
-- Up migration to create the play_types table
CREATE TABLE IF NOT EXISTS play_types (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  play_type TEXT NOT NULL UNIQUE
);
