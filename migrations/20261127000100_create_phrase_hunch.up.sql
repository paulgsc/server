-- A hunch no phrase does (paulgsc/server#1261, KPLAY's KP4; paulgsc/some-ui
-- #2853): the owner says, in English, what they want to say, and home's
-- meaning index finds the Korean phrases that do it
-- (`file_host::phrases`). When none is near enough, home keeps the hunch here,
-- in the request that answers `kept`, so a miss is never lost between two
-- calls, for the routine that writes new phrases (KP5).
--
--   hunch     -- the phone's name for the hunch: the shop runtime's id and
--                the hunch's number this session, the same each time it is
--                sent, so a hunch sent again is kept once.
--   words     -- what the owner typed, or what home heard them say: the
--                owner's own words, at most 200 characters.
--   asked_at  -- when home answered the hunch `kept`.
--
-- A spoken hunch's take is a voice note of kind `hunch` whose `ref` is the
-- same hunch id (voice_note, docs/identity.md invariant 23), so the take is
-- found by that, and nothing here points at it.
--
-- The owner's history: append-only (a trigger aborts an UPDATE), kept for
-- good, and deleted only with the account, by the sweep and by the trigger
-- below (docs/identity.md invariants 9, 12 and 41).
CREATE TABLE phrase_hunch (
    subject_id TEXT NOT NULL,
    hunch      TEXT NOT NULL CHECK (length(hunch) BETWEEN 1 AND 64 AND hunch NOT GLOB '*[^A-Za-z0-9_.:-]*'),
    words      TEXT NOT NULL CHECK (length(words) BETWEEN 1 AND 200),
    asked_at   TEXT NOT NULL,
    PRIMARY KEY (subject_id, hunch)
);

CREATE TRIGGER phrase_hunch_is_append_only
BEFORE UPDATE ON phrase_hunch
BEGIN
    SELECT RAISE(ABORT, 'phrase_hunch is append-only: a hunch is kept as it was asked');
END;

CREATE TRIGGER phrase_hunch_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM phrase_hunch WHERE subject_id = OLD.subject_id;
END;
