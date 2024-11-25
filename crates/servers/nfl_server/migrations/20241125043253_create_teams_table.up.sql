-- Add up migration script here
CREATE TABLE IF NOT EXISTS teams (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  abbreviation_id INTEGER NOT NULL,
  name_id INTEGER NOT NULL,
  stadium_id INTEGER NOT NULL,
  FOREIGN KEY (stadium_id) REFERENCES stadiums (id)
);
