-- Add up migration script here
CREATE TABLE IF NOT EXISTS mood_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  index_pos INTEGER NOT NULL,
  week INTEGER NOT NULL,
  label TEXT NOT NULL,
  description TEXT NOT NULL,
  team TEXT NOT NULL,
  category TEXT NOT NULL,
  delta INTEGER NOT NULL,
  mood INTEGER NOT NULL,
  created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
  updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
  UNIQUE (index_pos)
);

-- Indexes for faster queries
CREATE INDEX IF NOT EXISTS idx_mood_events_week ON mood_events (week);

CREATE INDEX IF NOT EXISTS idx_mood_events_team ON mood_events (team);

CREATE INDEX IF NOT EXISTS idx_mood_events_category ON mood_events (category);

CREATE INDEX IF NOT EXISTS idx_mood_events_index_pos ON mood_events (index_pos);
