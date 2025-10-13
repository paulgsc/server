-- Add up migration script here
CREATE TABLE IF NOT EXISTS nfl_games (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  encoded_date INTEGER NOT NULL,
  home_team_id INTEGER NOT NULL,
  away_team_id INTEGER NOT NULL,
  weather_id INTEGER NOT NULL,
  game_status_id INTEGER NOT NULL,
  FOREIGN KEY (home_team_id) REFERENCES teams (id),
  FOREIGN KEY (away_team_id) REFERENCES teams (id),
  FOREIGN KEY (weather_id) REFERENCES weather (id),
  CHECK (home_team_id != away_team_id)
);
