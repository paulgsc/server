-- Playoff Delta's fills, kept on home (FEED4: paulgsc/some-ui#2647, the plan
-- of 2026-11-07). The narration routine writes each week's fill (what is said
-- over each scene, `some-ui.pd-narration/1`) to `paulgsc/testing`, under
-- `playoff-delta/<season>/narration/`, and lists every fill in
-- `playoff-delta/index.json`. `file_host`'s `pd_fills` worker reads that index
-- on a timer and keeps each fill it does not hold yet, with the season file as
-- the repository served it then, so home can render the week again without
-- the repository. `pd_fill_repo` is the one module that writes this table.
--
-- Not subject-scoped: a fill says what happened in public NFL games, read
-- from ESPN's public scoreboards. No column names a person.
--
-- APPEND-ONLY: the triggers abort every `UPDATE` and `DELETE`. A fill is kept
-- once, by its episode (season, team, week), as the repository first served
-- it; a later edit there is not taken (the routine never edits a fill).
--
-- Columns:
--
--   season        -- the season's label, `YYYY`: its folder's name.
--   team          -- whose side the season is read from, `SF`.
--   week          -- the week, 0 for the legend.
--   version       -- the show's format version the fill was written for
--                    (`format.version`): 2 on is voiced by home's speaker.
--   sha256        -- lowercase hex SHA-256 of `body`.
--   size          -- `body`'s length in bytes.
--   body          -- the fill's file, exactly as the repository served it.
--   season_sha256 -- lowercase hex SHA-256 of `season_body`.
--   season_size   -- `season_body`'s length in bytes.
--   season_body   -- `<season>/<team>.json` as served when the fill was
--                    taken: the fill pins the season up to its week, which
--                    any later season file still holds.
--   taken_at      -- ISO-8601 UTC, when home stored it.
CREATE TABLE pd_fill (
    season        TEXT    NOT NULL CHECK (season GLOB '[0-9][0-9][0-9][0-9]'),
    team          TEXT    NOT NULL CHECK (length(team) BETWEEN 2 AND 3 AND team NOT GLOB '*[^A-Z]*'),
    week          INTEGER NOT NULL CHECK (week BETWEEN 0 AND 99),
    version       INTEGER NOT NULL CHECK (version >= 1),
    sha256        TEXT    NOT NULL CHECK (length(sha256) = 64),
    size          INTEGER NOT NULL CHECK (size = length(body)),
    body          BLOB    NOT NULL,
    season_sha256 TEXT    NOT NULL CHECK (length(season_sha256) = 64),
    season_size   INTEGER NOT NULL CHECK (season_size = length(season_body)),
    season_body   BLOB    NOT NULL,
    taken_at      TEXT    NOT NULL,
    PRIMARY KEY (season, team, week)
);

CREATE TRIGGER pd_fill_no_update
BEFORE UPDATE ON pd_fill
BEGIN
    SELECT RAISE(ABORT, 'pd_fill is append-only: a kept fill is never rewritten');
END;

CREATE TRIGGER pd_fill_no_delete
BEFORE DELETE ON pd_fill
BEGIN
    SELECT RAISE(ABORT, 'pd_fill is append-only: a kept fill is never deleted');
END;
