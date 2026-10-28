-- The NFL game feed (paulgsc/some-ui#2076, GWB2): every response home
-- fetched from ESPN's site API, and the play log read from them.
-- `file_host`'s `game_feed` worker is the one writer; `game_feed_repo` is
-- the one module that writes these tables.
--
-- Not subject-scoped: public game data, nobody's record. No column names a
-- person, and nothing here is read per subject.
--
-- APPEND-ONLY, both tables (GW10: any game can be replayed later and give
-- the same result). The triggers abort every `UPDATE` and `DELETE`; nothing
-- writes `INSERT OR REPLACE`.

-- Every response, as it arrived, whatever it was: a scoreboard, a summary,
-- a 429, an HTML error page. Columns:
--
--   response_id -- order of arrival.
--   kind        -- `scoreboard` or `summary`.
--   game_id     -- for `summary`: the ESPN event asked for. NULL otherwise.
--   url         -- the URL fetched (ESPN's public API; no secret in it).
--   fetched_at  -- ISO-8601 UTC, when the response arrived.
--   status      -- the HTTP status.
--   sha256      -- lowercase hex SHA-256 of `body`.
--   size        -- `body`'s length in bytes.
--   body        -- the bytes, exactly as they arrived.
CREATE TABLE game_feed_response (
    response_id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind        TEXT    NOT NULL CHECK (kind IN ('scoreboard', 'summary')),
    game_id     TEXT,
    url         TEXT    NOT NULL,
    fetched_at  TEXT    NOT NULL,
    status      INTEGER NOT NULL CHECK (status BETWEEN 100 AND 599),
    sha256      TEXT    NOT NULL CHECK (length(sha256) = 64),
    size        INTEGER NOT NULL CHECK (size = length(body)),
    body        BLOB    NOT NULL,
    CHECK ((kind = 'summary') = (game_id IS NOT NULL))
);

-- A game's summaries, newest first, for the worker's restart check.
CREATE INDEX game_feed_response_by_game ON game_feed_response (game_id, response_id) WHERE game_id IS NOT NULL;

CREATE TRIGGER game_feed_response_no_update
BEFORE UPDATE ON game_feed_response
BEGIN
    SELECT RAISE(ABORT, 'game_feed_response is append-only: a kept response is never rewritten');
END;

CREATE TRIGGER game_feed_response_no_delete
BEFORE DELETE ON game_feed_response
BEGIN
    SELECT RAISE(ABORT, 'game_feed_response is append-only: a kept response is never deleted');
END;

-- The play log. One row per version of a play: the first time home sees a
-- play, and again each time ESPN revises it, the revision pointing at the
-- row it supersedes. A play seen again unchanged adds nothing. Columns, the
-- play's as `nfl_feed::summary::Play` documents them:
--
--   row_id           -- order of addition.
--   game_id          -- the ESPN event.
--   espn_play_id     -- ESPN's play id, the same across revisions.
--   sequence         -- ESPN's sequence number.
--   wallclock        -- when the play happened, as ESPN reports it.
--   period, clock    -- quarter (5 is overtime) and game clock at the snap.
--   down, distance   -- at the snap; down NULL where ESPN says 0.
--   yard_line        -- ESPN's 0-100 yard line at the snap.
--   yards_to_endzone -- for the team with the ball, at the snap.
--   possession       -- that team's abbreviation.
--   play_type        -- ESPN's play type in words.
--   scoring          -- 1 for a scoring play.
--   away_score, home_score -- after the play.
--   text             -- ESPN's description.
--   response_id      -- the kept response this version was read from.
--   supersedes       -- the row this version revises; NULL for the first.
--   seen_at          -- when home first saw this version: that response's
--                       `fetched_at`, so a replay gives the same value.
CREATE TABLE game_feed_play (
    row_id           INTEGER PRIMARY KEY AUTOINCREMENT,
    game_id          TEXT    NOT NULL,
    espn_play_id     TEXT    NOT NULL,
    sequence         INTEGER,
    wallclock        TEXT,
    period           INTEGER,
    clock            TEXT,
    down             INTEGER,
    distance         INTEGER,
    yard_line        INTEGER,
    yards_to_endzone INTEGER,
    possession       TEXT,
    play_type        TEXT,
    scoring          INTEGER CHECK (scoring IN (0, 1)),
    away_score       INTEGER,
    home_score       INTEGER,
    text             TEXT,
    response_id      INTEGER NOT NULL REFERENCES game_feed_response (response_id),
    supersedes       INTEGER REFERENCES game_feed_play (row_id),
    seen_at          TEXT    NOT NULL
);

-- One first version per play, and each version revised at most once, so a
-- play's versions are one chain and its latest is the row nothing
-- supersedes.
CREATE UNIQUE INDEX game_feed_play_first ON game_feed_play (game_id, espn_play_id) WHERE supersedes IS NULL;
CREATE UNIQUE INDEX game_feed_play_revised_once ON game_feed_play (supersedes) WHERE supersedes IS NOT NULL;
CREATE INDEX game_feed_play_by_game ON game_feed_play (game_id, row_id);

-- A revision revises the same play.
CREATE TRIGGER game_feed_play_revises_its_own_play
BEFORE INSERT ON game_feed_play
WHEN NEW.supersedes IS NOT NULL
 AND NOT EXISTS (SELECT 1 FROM game_feed_play
                 WHERE row_id = NEW.supersedes AND game_id = NEW.game_id AND espn_play_id = NEW.espn_play_id)
BEGIN
    SELECT RAISE(ABORT, 'game_feed_play: a revision must supersede a row of the same play');
END;

CREATE TRIGGER game_feed_play_no_update
BEFORE UPDATE ON game_feed_play
BEGIN
    SELECT RAISE(ABORT, 'game_feed_play is append-only: a revision is a new row');
END;

CREATE TRIGGER game_feed_play_no_delete
BEFORE DELETE ON game_feed_play
BEGIN
    SELECT RAISE(ABORT, 'game_feed_play is append-only: a play is never deleted');
END;
