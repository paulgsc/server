-- The reconciler (paulgsc/some-ui#2081, GWB7): each game bite matched to the
-- play it was about, and the scenarios home mapped to bites said first. Both
-- tables are append-only (the epic's rule: never lose the record): a later
-- pass adds a row, and a reader takes a bite's newest. A bite itself is never
-- rewritten (`game_bite_is_append_only`).
--
-- Neither names `game_bite` by a foreign key, as `game_bite` names no
-- `voice_note`: a key would make a later rebuild of `game_bite` (as
-- 20261028000300 did) fail on its rows. Both leave with the account instead.
--
-- `bite_match`, one row per pass that changed what a bite matched:
--
--   match_id       -- order of addition; a bite's newest row is its match.
--   subject_id     -- whose bite.
--   note_id        -- which bite: `game_bite`'s, keyed as the note is.
--   scenario_id    -- the scenario it was matched under: the bite's own, or
--                     else its newest `bite_mapping`'s; NULL for a bite said
--                     first that nothing mapped.
--   play_id        -- ESPN's play id in the bite's game (`game_feed_play`'s
--                     `espn_play_id`), or NULL when no play was matched.
--   method         -- `nearest` (the nearest play the scenario's rule accepts,
--                     at or before the bite, within the window), `unmatched`
--                     (no such play: kept, GW15), `not-a-play` (the scenario
--                     is about a stretch of the game: pregame, the two-minute
--                     drill, the final, the owner's own) or `no-scenario` (said
--                     first and not mapped).
--   gap_seconds    -- the bite's start on home's clock minus the play's time;
--                     NULL exactly when no play was matched.
--   engine_version -- the scenario engine's rules the match was made under.
--   matched_at     -- when home made it.
CREATE TABLE bite_match (
    match_id       INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_id     TEXT    NOT NULL,
    note_id        TEXT    NOT NULL,
    scenario_id    TEXT,
    play_id        TEXT,
    method         TEXT    NOT NULL CHECK (method IN ('nearest', 'unmatched', 'not-a-play', 'no-scenario')),
    gap_seconds    INTEGER CHECK (gap_seconds >= 0),
    engine_version INTEGER NOT NULL CHECK (engine_version >= 1),
    matched_at     TEXT    NOT NULL,
    -- A play exactly when matched, and a gap exactly with a play.
    CHECK ((method = 'nearest') = (play_id IS NOT NULL)),
    CHECK ((play_id IS NULL) = (gap_seconds IS NULL)),
    -- A scenario unless there was none to match under.
    CHECK ((method = 'no-scenario') = (scenario_id IS NULL))
);

-- A bite's newest match: an indexed range read.
CREATE INDEX bite_match_by_bite ON bite_match (subject_id, note_id, match_id);

CREATE TRIGGER bite_match_is_append_only
BEFORE UPDATE ON bite_match
BEGIN
    SELECT RAISE(ABORT, 'bite_match is append-only: a later match is a new row');
END;

-- `bite_mapping`, one row per accepted proposal for a bite said first. A
-- proposal is accepted only if the scenario is in home's set and replaying the
-- engine at the bite's time shows it could have been happening; a rejected
-- one adds nothing, so a bite is left unmapped, never mislabelled.
--
--   mapping_id      -- order of addition; a bite's newest row is its mapping.
--   subject_id, note_id -- which bite, as in `bite_match`.
--   scenario_id     -- the scenario accepted, as the set names it.
--   proposed_by     -- what proposed it: `shot-writer` (GWB9).
--   catalog_version -- the scenario set it was checked against.
--   engine_version  -- the engine's rules the replay ran under.
--   mapped_at       -- when home accepted it.
CREATE TABLE bite_mapping (
    mapping_id      INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_id      TEXT    NOT NULL,
    note_id         TEXT    NOT NULL,
    scenario_id     TEXT    NOT NULL,
    proposed_by     TEXT    NOT NULL CHECK (proposed_by IN ('shot-writer')),
    catalog_version INTEGER NOT NULL CHECK (catalog_version >= 1),
    engine_version  INTEGER NOT NULL CHECK (engine_version >= 1),
    mapped_at       TEXT    NOT NULL
);

CREATE INDEX bite_mapping_by_bite ON bite_mapping (subject_id, note_id, mapping_id);

CREATE TRIGGER bite_mapping_is_append_only
BEFORE UPDATE ON bite_mapping
BEGIN
    SELECT RAISE(ABORT, 'bite_mapping is append-only: a later mapping is a new row');
END;

-- The reconciler reads one game's bites: the game notes whose `ref` is that
-- game, an indexed range read rather than a scan of every note. A later
-- rebuild of `voice_note` recreates this index with the others.
CREATE INDEX voice_note_by_ref ON voice_note (of_kind, of_ref);

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `game_bite` does.
CREATE TRIGGER bite_match_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM bite_match WHERE subject_id = OLD.subject_id;
    DELETE FROM bite_mapping WHERE subject_id = OLD.subject_id;
END;
