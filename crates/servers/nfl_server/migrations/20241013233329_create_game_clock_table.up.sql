-- Add up migration script here
CREATE TABLE IF NOT EXISTS game_clock (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  minutes INTEGER NOT NULL CHECK (minutes BETWEEN 0 AND 15),
  seconds INTEGER NOT NULL CHECK (seconds BETWEEN 0 AND 59)
);
