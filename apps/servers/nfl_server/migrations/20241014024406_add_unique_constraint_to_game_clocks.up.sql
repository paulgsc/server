-- Rename the existing table
ALTER TABLE game_clock RENAME TO game_clock_old;

-- Create a new table with the desired structure and constraint
CREATE TABLE game_clock (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    minutes INTEGER NOT NULL,
    seconds INTEGER NOT NULL,
    UNIQUE(minutes, seconds)
);

-- Copy the data from the old table to the new one
INSERT INTO game_clock (minutes, seconds)
SELECT minutes, seconds FROM game_clock_old;

-- Drop the old table
DROP TABLE game_clock_old;
