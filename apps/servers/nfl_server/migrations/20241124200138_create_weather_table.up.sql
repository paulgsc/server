-- Add up migration script here
CREATE TABLE IF NOT EXISTS weather (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  condition INTEGER NOT NULL,
  day_night INTEGER NOT NULL,
  temperature REAL NOT NULL,
  wind_speed REAL,
  CHECK (temperature BETWEEN -50.0 AND 150.0),
  CHECK (
    wind_speed IS NULL
    OR (wind_speed BETWEEN 0.0 AND 200.0)
  )
);
