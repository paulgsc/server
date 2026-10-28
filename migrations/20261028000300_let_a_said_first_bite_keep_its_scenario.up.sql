-- A bite said first may keep the scenario the owner picked after it
-- (paulgsc/some-ui#2080, GWB6). On the phone, "Say it first" is: talk, then
-- pick what it was about from what could be happening then, or leave it for
-- later (home maps it). `how` still records that it was said first (GW4:
-- every bite carries how it was chosen), so a said-first bite's
-- `scenario_id` is the scenario picked after speaking, or NULL when it was
-- left for later and is not yet mapped. Every other `how` still names its
-- scenario.
--
-- 20261028000100_add_game_bite_to_voice_note has shipped, so it is not
-- edited: SQLite cannot alter a CHECK, so `game_bite` is rebuilt as that
-- migration rebuilt `voice_note`: a fresh `game_bite_new` with the relaxed
-- CHECK, every row copied across unchanged, the old table dropped and the
-- new one renamed in. Only the CHECK between `how` and `scenario_id`
-- changes; every column, its own CHECK, the primary key, the append-only
-- trigger and the account trigger are recreated word for word. The account
-- trigger names `game_bite` in its body, so it is dropped first and
-- recreated once the rebuilt table holds the name again; the append-only
-- trigger goes with the dropped table. No index, view or other trigger
-- names `game_bite` (every migration grepped).
DROP TRIGGER game_bite_leaves_with_account;

CREATE TABLE game_bite_new (
    subject_id      TEXT    NOT NULL,
    note_id         TEXT    NOT NULL,
    scenario_id     TEXT,
    how             TEXT    NOT NULL CHECK (how IN ('offered', 'just-happened', 'said-first', 'every-scenario')),
    offer_seq       INTEGER CHECK (offer_seq >= 0),
    live_strip      TEXT    NOT NULL,
    context_line    TEXT,
    watching        INTEGER NOT NULL CHECK (watching IN (0, 1)),
    started_at      TEXT    NOT NULL,
    clock_offset_ms INTEGER,
    mode            TEXT    NOT NULL CHECK (mode IN ('light', 'standard', 'full')),
    catalog_version INTEGER NOT NULL CHECK (catalog_version >= 1),
    PRIMARY KEY (subject_id, note_id),
    -- A scenario unless the bite was said first; said first, the scenario
    -- picked after it, or none while it is left for later.
    CHECK (how = 'said-first' OR scenario_id IS NOT NULL)
);

INSERT INTO game_bite_new (subject_id, note_id, scenario_id, how, offer_seq, live_strip, context_line, watching,
                           started_at, clock_offset_ms, mode, catalog_version)
SELECT subject_id, note_id, scenario_id, how, offer_seq, live_strip, context_line, watching,
       started_at, clock_offset_ms, mode, catalog_version FROM game_bite;

DROP TABLE game_bite;
ALTER TABLE game_bite_new RENAME TO game_bite;

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
