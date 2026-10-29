-- The game feed stores each ESPN body once, by content (paulgsc/server#544,
-- STORAGE4). Decided by the owner on 2026-10-28: dedupe, only. Every fetch
-- still keeps its `game_feed_response` row (GW10: any game can be replayed);
-- what moves is the bytes. Most consecutive live fetches return identical
-- bytes (ESPN caches for 5 s and a play comes about every 40 s), so the
-- feed keeps the unique bodies per followed game, not one body per fetch.
--
-- `game_feed_repo` is still the one module that writes these tables.
--
-- APPEND-ONLY, all three tables. `game_feed_body`'s triggers abort every
-- `UPDATE` and `DELETE`, and an insert of a hash already held is skipped
-- (`RAISE(IGNORE)`), whatever its conflict clause: `INSERT OR IGNORE` is how
-- it is written, and an `INSERT OR REPLACE` would otherwise delete the held
-- row without firing a `DELETE` trigger (`recursive_triggers` is off).
--
-- Every body, once. Columns:
--
--   sha256 -- lowercase hex SHA-256 of `body`; the key a response names.
--   body   -- the bytes, exactly as they arrived.
--
-- A rowid table, not `WITHOUT ROWID`: a body is 0.1-0.56 MB, far past the
-- row size `WITHOUT ROWID` suits.
CREATE TABLE game_feed_body (
    sha256 TEXT NOT NULL PRIMARY KEY CHECK (length(sha256) = 64 AND sha256 NOT GLOB '*[^0-9a-f]*'),
    body   BLOB NOT NULL
);

CREATE TRIGGER game_feed_body_keeps_the_first
BEFORE INSERT ON game_feed_body
WHEN EXISTS (SELECT 1 FROM game_feed_body WHERE sha256 = NEW.sha256)
BEGIN
    SELECT RAISE(IGNORE);
END;

CREATE TRIGGER game_feed_body_no_update
BEFORE UPDATE ON game_feed_body
BEGIN
    SELECT RAISE(ABORT, 'game_feed_body is append-only: a kept body is never rewritten');
END;

CREATE TRIGGER game_feed_body_no_delete
BEFORE DELETE ON game_feed_body
BEGIN
    SELECT RAISE(ABORT, 'game_feed_body is append-only: a kept body is never deleted');
END;

-- How the rows already held move. The feed is off by default, so most homes
-- hold none; where it has run, every row is kept, with its `response_id`.
--
-- 1. Every body goes into `game_feed_body`, oldest response first. A hash
--    held twice was computed from the same bytes, and the check below
--    aborts the migration if any response's bytes differ from the body its
--    hash now names, so no body is lost to a collision or a bad hash.
INSERT OR IGNORE INTO game_feed_body (sha256, body)
SELECT sha256, body FROM game_feed_response ORDER BY response_id;

CREATE TEMP TABLE game_feed_body_check (mismatched INTEGER NOT NULL CHECK (mismatched = 0));
INSERT INTO game_feed_body_check (mismatched)
SELECT COUNT(*) FROM game_feed_response r JOIN game_feed_body b ON b.sha256 = r.sha256 WHERE b.body IS NOT r.body;
DROP TABLE game_feed_body_check;

-- 2. SQLite drops a column only by rebuilding the table (`size`'s CHECK
--    names `body`, so `ALTER TABLE DROP COLUMN` refuses it). The rows wait
--    in a staging table, with the table's AUTOINCREMENT high-water mark, so
--    no id is ever reused.
CREATE TEMP TABLE game_feed_response_moving AS
SELECT response_id, kind, game_id, url, fetched_at, status, sha256, size FROM game_feed_response;

CREATE TEMP TABLE game_feed_response_sequence AS
SELECT seq FROM sqlite_sequence WHERE name = 'game_feed_response';

-- 3. `game_feed_play.response_id` references `game_feed_response` by name.
--    Dropping the table deletes its rows implicitly, and with the foreign
--    keys deferred (until this migration's transaction commits) that
--    counts every play row as orphaned instead of failing at once. Each row
--    put back below finds its plays again and takes them off the count, so
--    the commit fails if any play's response did not come back. Renaming
--    instead would rewrite `game_feed_play`'s reference to follow the old
--    table.
PRAGMA defer_foreign_keys = ON;

DROP INDEX game_feed_response_by_game;
DROP TRIGGER game_feed_response_no_update;
DROP TRIGGER game_feed_response_no_delete;
DROP TABLE game_feed_response;

-- Every response, as it arrived, its bytes in `game_feed_body`. Columns:
--
--   response_id -- order of arrival.
--   kind        -- `scoreboard` or `summary`.
--   game_id     -- for `summary`: the ESPN event asked for. NULL otherwise.
--   url         -- the URL fetched (ESPN's public API; no secret in it).
--   fetched_at  -- ISO-8601 UTC, when the response arrived.
--   status      -- the HTTP status.
--   sha256      -- lowercase hex SHA-256 of the body: its `game_feed_body` key.
--   size        -- the body's length in bytes.
CREATE TABLE game_feed_response (
    response_id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind        TEXT    NOT NULL CHECK (kind IN ('scoreboard', 'summary')),
    game_id     TEXT,
    url         TEXT    NOT NULL,
    fetched_at  TEXT    NOT NULL,
    status      INTEGER NOT NULL CHECK (status BETWEEN 100 AND 599),
    sha256      TEXT    NOT NULL REFERENCES game_feed_body (sha256),
    size        INTEGER NOT NULL CHECK (size >= 0),
    CHECK ((kind = 'summary') = (game_id IS NOT NULL))
);

INSERT INTO game_feed_response (response_id, kind, game_id, url, fetched_at, status, sha256, size)
SELECT response_id, kind, game_id, url, fetched_at, status, sha256, size
FROM game_feed_response_moving ORDER BY response_id;

-- Every row came back, the ones no play names included.
CREATE TEMP TABLE game_feed_response_check (missing INTEGER NOT NULL CHECK (missing = 0));
INSERT INTO game_feed_response_check (missing)
SELECT (SELECT COUNT(*) FROM game_feed_response_moving) - (SELECT COUNT(*) FROM game_feed_response);
DROP TABLE game_feed_response_check;

UPDATE sqlite_sequence
SET seq = max(seq, coalesce((SELECT seq FROM game_feed_response_sequence), 0))
WHERE name = 'game_feed_response';

DROP TABLE game_feed_response_sequence;
DROP TABLE game_feed_response_moving;

-- A game's summaries, newest first, for the worker's restart check.
CREATE INDEX game_feed_response_by_game ON game_feed_response (game_id, response_id) WHERE game_id IS NOT NULL;

-- A response names a body already kept, and its `size` is that body's
-- length: what `size`'s CHECK held while the bytes were in this table.
CREATE TRIGGER game_feed_response_names_its_body
BEFORE INSERT ON game_feed_response
WHEN NEW.size IS NOT (SELECT length(body) FROM game_feed_body WHERE sha256 = NEW.sha256)
BEGIN
    SELECT RAISE(ABORT, 'game_feed_response: its body is kept first, and size is that body''s length');
END;

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
