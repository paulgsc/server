-- Rename the current table
ALTER TABLE game_clock RENAME TO game_clock_temp;

-- Recreate the original table without the unique constraint
CREATE TABLE game_clock (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    minutes INTEGER NOT NULL,
    seconds INTEGER NOT NULL
);

-- Copy data back
INSERT INTO game_clock (id, minutes, seconds)
SELECT id, minutes, seconds FROM game_clock_temp;

-- Drop the temporary table
DROP TABLE game_clock_temp;
