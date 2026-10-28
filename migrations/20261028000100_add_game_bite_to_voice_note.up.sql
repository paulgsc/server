-- Game bites are voice notes (paulgsc/some-ui#2079, GWB5; the owner's
-- decision on paulgsc/some-ui#2074, "Decided" 1, 2026-10-26): a bite is a
-- voice note the owner says on the phone while watching an NFL game, about
-- one scenario of a fixed set. Its audio is a note like any other, so it is
-- kept as `voice_note` and `voice_note_audio` keep every note (append-only,
-- kept for good, docs/identity.md invariant 23), said for `game`, its `ref`
-- the game's id; what the bite was said about and what the screen showed
-- sits beside it in `game_bite`, written in the same transaction.
--
-- `voice_note` takes a fourth "said for", `game`. SQLite cannot alter a
-- CHECK, so it is rebuilt as 20261026000300_add_korean_to_sharing_choice
-- rebuilt `sharing_choice`: a fresh `voice_note_new` with the wider CHECK,
-- every row copied across unchanged, the old table dropped and the new one
-- renamed in. Only the CHECK on `of_kind` widens; every column, the append-
-- only trigger, the transcriber's index (`voice_note_by_stored_at`, from
-- 20261025000200) and the account trigger are recreated word for word. The
-- account trigger names `voice_note` in its body, so it is dropped first and
-- recreated once the rebuilt table holds the name again. `voice_note_audio`,
-- `voice_transcript` and `mailbox_transcript` name a note by its id only and
-- are untouched.
--
-- Columns of `game_bite` (one row per `game` note, keyed as the note is):
--
--   subject_id      -- whose bite.
--   note_id         -- which note: `voice_note`'s id.
--   scenario_id     -- the scenario it was said about, as the phone's copy
--                      of the catalogue (crate `game_scenarios`) names it
--                      (`in.third-long`); NULL exactly when it was said first,
--                      before any scenario.
--   how             -- how the scenario came to be said: `offered` (the
--                      screen offered it), `just-happened` (the owner said
--                      what just happened), `said-first` (said before
--                      choosing a scenario) or `every-scenario` (chosen from
--                      the whole set).
--   offer_seq       -- which offer the screen was showing, when it showed one.
--   live_strip      -- the live strip as the screen showed it: the owner's
--                      game as text (score, clock, down). Never logged.
--   context_line    -- the line of context the screen showed, if any. Never
--                      logged.
--   game_id         -- the game: ESPN's event id, as the phone names it; the
--                      note's `of_ref`.
--   watching        -- 1 when the owner said they were watching the game, 0
--                      when not.
--   started_at      -- when the owner started saying it, on the phone's
--                      clock, as ISO-8601 UTC to the millisecond.
--   clock_offset_ms -- home's clock minus the phone's, in milliseconds, from
--                      the live connection; NULL while unknown.
--   mode            -- how much the screen offered: `light`, `standard` or
--                      `full`.
--   catalog_version -- the version of the scenario set it was said under.
DROP TRIGGER voice_note_leaves_with_account;

CREATE TABLE voice_note_new (
    subject_id   TEXT    NOT NULL,
    note_id      TEXT    NOT NULL,
    of_kind      TEXT    NOT NULL CHECK (of_kind IN ('reflection', 'jobs', 'scene', 'game')),
    of_ref       TEXT    NOT NULL,
    recorded_at  TEXT    NOT NULL,
    content_type TEXT    NOT NULL CHECK (content_type IN ('audio/webm', 'audio/ogg', 'audio/mp4')),
    size         INTEGER NOT NULL CHECK (size > 0),
    sha256       TEXT    NOT NULL,
    stored_at    TEXT    NOT NULL,
    PRIMARY KEY (subject_id, note_id)
);

INSERT INTO voice_note_new (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
SELECT subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at FROM voice_note;

DROP TABLE voice_note;
ALTER TABLE voice_note_new RENAME TO voice_note;

CREATE TRIGGER voice_note_is_append_only
BEFORE UPDATE ON voice_note
BEGIN
    SELECT RAISE(ABORT, 'voice_note is append-only: a note is never rewritten');
END;

-- The worker takes the oldest notes first.
CREATE INDEX voice_note_by_stored_at ON voice_note (stored_at, subject_id, note_id);

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `jobs_event` does.
CREATE TRIGGER voice_note_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM voice_note WHERE subject_id = OLD.subject_id;
    DELETE FROM voice_note_audio WHERE subject_id = OLD.subject_id;
END;

CREATE TABLE game_bite (
    subject_id      TEXT    NOT NULL,
    note_id         TEXT    NOT NULL,
    scenario_id     TEXT,
    how             TEXT    NOT NULL CHECK (how IN ('offered', 'just-happened', 'said-first', 'every-scenario')),
    offer_seq       INTEGER CHECK (offer_seq >= 0),
    live_strip      TEXT    NOT NULL,
    context_line    TEXT,
    game_id         TEXT    NOT NULL,
    watching        INTEGER NOT NULL CHECK (watching IN (0, 1)),
    started_at      TEXT    NOT NULL,
    clock_offset_ms INTEGER,
    mode            TEXT    NOT NULL CHECK (mode IN ('light', 'standard', 'full')),
    catalog_version INTEGER NOT NULL CHECK (catalog_version >= 1),
    PRIMARY KEY (subject_id, note_id),
    -- A scenario exactly when the bite was not said first.
    CHECK ((how = 'said-first') = (scenario_id IS NULL))
);

CREATE TRIGGER game_bite_is_append_only
BEFORE UPDATE ON game_bite
BEGIN
    SELECT RAISE(ABORT, 'game_bite is append-only: a bite is never rewritten');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `voice_note` does.
CREATE TRIGGER game_bite_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM game_bite WHERE subject_id = OLD.subject_id;
END;
