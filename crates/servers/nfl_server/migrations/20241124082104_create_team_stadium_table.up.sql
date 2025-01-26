-- Add up migration script here
CREATE TABLE IF NOT EXISTS stadiums (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL,
  state INTEGER NOT NULL,
  city TEXT NOT NULL,
  stadium_type INTEGER NOT NULL,
  surface_type INTEGER NOT NULL,
  capacity INTEGER NOT NULL,
  CONSTRAINT name_unique UNIQUE (name)
);
