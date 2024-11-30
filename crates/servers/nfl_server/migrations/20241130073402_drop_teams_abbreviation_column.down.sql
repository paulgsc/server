-- Add down migration script here
ALTER TABLE teams
ADD COLUMN abbreviation_id INTEGER NOT NULL;
