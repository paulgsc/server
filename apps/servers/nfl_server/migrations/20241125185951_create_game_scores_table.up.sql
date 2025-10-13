CREATE TABLE IF NOT EXISTS game_scores (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  game_id INTEGER NOT NULL,
  team_id INTEGER NOT NULL,
  scoring_event INTEGER NOT NULL,
  quarter INTEGER NOT NULL CHECK (quarter BETWEEN 1 AND 5),
  clock_id INTEGER NOT NULL,
  FOREIGN KEY (game_id) REFERENCES nfl_games (id),
  FOREIGN KEY (team_id) REFERENCES teams (id),
  FOREIGN KEY (clock_id) REFERENCES game_clock (id),
  UNIQUE (game_id, team_id, quarter, clock_id)
);
