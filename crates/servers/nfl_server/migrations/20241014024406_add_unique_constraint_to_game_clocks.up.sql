-- Add up migration script here
ALTER TABLE game_clocks
ADD CONSTRAINT unique_minutes_seconds UNIQUE (minutes, seconds);

