-- What a game bite is about (paulgsc/some-ui#3049, GWB18): which unit, and
-- whose. `unit` is `offense`, `defense`, `special-teams` or `coaching`
-- (`game_scenarios::Unit`); `unit_side` is the team's code as ESPN writes it
-- (`CHI`), or NULL when the bite is about no one team. The phone defaults
-- both from the moment and the owner changes them with one tap; home keeps
-- what the phone said, once, with the bite (a later correction would be a
-- record of its own, never an edit). NULL for a bite that said none (a phone
-- from before, or a bite before kickoff).
--
-- Additive: SQLite adds each column with its CHECK in place, so no rebuild.
-- A side is only ever said with a unit. A bite is still never rewritten
-- (`game_bite_is_append_only`); both are part of what a replay must match,
-- as the scenario is.
ALTER TABLE game_bite ADD COLUMN unit TEXT CHECK (unit IN ('offense', 'defense', 'special-teams', 'coaching'));
ALTER TABLE game_bite ADD COLUMN unit_side TEXT
    CHECK (unit_side IS NULL OR (unit IS NOT NULL AND length(unit_side) BETWEEN 2 AND 4 AND unit_side NOT GLOB '*[^A-Z]*'));
