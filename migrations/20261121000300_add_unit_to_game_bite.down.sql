-- Reverses 20261121000300_add_unit_to_game_bite.up.sql.
--
-- **It refuses rather than discards,** as 20261028000300's down does: once a
-- bite holds a unit, dropping the columns would lose what the owner said, so
-- the insert below fails its CHECK on any such row and the migration's
-- transaction leaves the schema as it was.
CREATE TABLE game_bite_unit_refusal (held INTEGER CHECK (held = 0));
INSERT INTO game_bite_unit_refusal (held) SELECT COUNT(*) FROM game_bite WHERE unit IS NOT NULL;
DROP TABLE game_bite_unit_refusal;
ALTER TABLE game_bite DROP COLUMN unit_side;
ALTER TABLE game_bite DROP COLUMN unit;
