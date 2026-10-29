-- Puts each response's bytes back in its own row, every row and id kept,
-- then drops `game_feed_body`. The same rebuild as the up migration, the
-- other way: see there for why the foreign keys are deferred.
CREATE TEMP TABLE game_feed_response_moving AS
SELECT r.response_id, r.kind, r.game_id, r.url, r.fetched_at, r.status, r.sha256, r.size, b.body
FROM game_feed_response r JOIN game_feed_body b ON b.sha256 = r.sha256;

-- Every response found its body.
CREATE TEMP TABLE game_feed_body_check (missing INTEGER NOT NULL CHECK (missing = 0));
INSERT INTO game_feed_body_check (missing)
SELECT (SELECT COUNT(*) FROM game_feed_response) - (SELECT COUNT(*) FROM game_feed_response_moving);
DROP TABLE game_feed_body_check;

CREATE TEMP TABLE game_feed_response_sequence AS
SELECT seq FROM sqlite_sequence WHERE name = 'game_feed_response';

PRAGMA defer_foreign_keys = ON;

DROP INDEX game_feed_response_by_game;
DROP TRIGGER game_feed_response_names_its_body;
DROP TRIGGER game_feed_response_no_update;
DROP TRIGGER game_feed_response_no_delete;
DROP TABLE game_feed_response;

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

INSERT INTO game_feed_response (response_id, kind, game_id, url, fetched_at, status, sha256, size, body)
SELECT response_id, kind, game_id, url, fetched_at, status, sha256, size, body
FROM game_feed_response_moving ORDER BY response_id;

UPDATE sqlite_sequence
SET seq = max(seq, coalesce((SELECT seq FROM game_feed_response_sequence), 0))
WHERE name = 'game_feed_response';

DROP TABLE game_feed_response_sequence;
DROP TABLE game_feed_response_moving;

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

DROP TRIGGER game_feed_body_no_delete;
DROP TRIGGER game_feed_body_no_update;
DROP TRIGGER game_feed_body_keeps_the_first;
DROP TABLE game_feed_body;
