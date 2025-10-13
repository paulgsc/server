-- Add up migration script here
CREATE TABLE IF NOT EXISTS player_dobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    dob_encoded INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_player_dobs_encoded
ON player_dobs(dob_encoded);
