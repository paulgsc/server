-- Add down migration script here
-- Create a temporary table without the unique constraint
CREATE TABLE game_clocks_temp (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    minutes INTEGER NOT NULL,
    seconds INTEGER NOT NULL
);

-- Copy data from the old table to the new table
INSERT INTO game_clocks_temp (id, minutes, seconds)
SELECT id, minutes, seconds FROM game_clocks;

-- Drop the old table
DROP TABLE game_clocks;

-- Rename the new table to the old table name
ALTER TABLE game_clocks_temp RENAME TO game_clocks;

