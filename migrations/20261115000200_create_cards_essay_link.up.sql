-- A cards essay's YouTube link (paulgsc/some-ui#2999, call C388), as Playoff
-- Delta's is (`pd_video_link`): the owner watches a done essay on the phone,
-- shares it to the YouTube app, and pastes its link
-- (`PUT /feeds/cards-essay/videos/:key/link`). `cards_essay` is append-only
-- and holds the essay alone, so the link is kept beside it here.
-- `cards_essay_repo` is the one module that writes this table.
--
-- Letting home's copy go once the link is recorded is a later story: there is
-- no receipt table yet, and `DELETE …/video` and `…/rest` still answer a cards
-- essay `404` (`feeds::Feed::lets_go`).
--
-- Not subject-scoped, as `cards_essay` is not: an essay explains public
-- repositories, and its video is public on YouTube. No column names a person.
--
-- APPEND-ONLY: the triggers abort every `UPDATE` and `DELETE`. A link is kept
-- once per essay and video; the newest recorded is the essay's.
--
--   essay_key   -- the essay's key, as `cards_essay` holds it.
--   youtube_id  -- the video's 11-character id, as `pd_video_link`'s.
--   recorded_at -- ISO-8601 UTC, when home recorded it.
CREATE TABLE cards_essay_link (
    essay_key   TEXT NOT NULL REFERENCES cards_essay (essay_key),
    youtube_id  TEXT NOT NULL CHECK (length(youtube_id) = 11 AND youtube_id NOT GLOB '*[^A-Za-z0-9_-]*'),
    recorded_at TEXT NOT NULL,
    PRIMARY KEY (essay_key, youtube_id)
);

CREATE TRIGGER cards_essay_link_no_update
BEFORE UPDATE ON cards_essay_link
BEGIN
    SELECT RAISE(ABORT, 'cards_essay_link is append-only: a recorded link is never rewritten');
END;

CREATE TRIGGER cards_essay_link_no_delete
BEFORE DELETE ON cards_essay_link
BEGIN
    SELECT RAISE(ABORT, 'cards_essay_link is append-only: a recorded link is never deleted');
END;
