-- Reverses 20261028000300_let_a_said_first_bite_keep_its_scenario.up.sql:
-- rebuilds `game_bite` back to the CHECK 20261028000100 declared (a
-- scenario exactly when the bite was not said first), the same way.
--
-- **It refuses rather than discards,** as 20261028000100's down does: every
-- bite is copied into a table whose CHECK refuses a said-first bite with a
-- scenario, so once one is held this fails on that row and the migration's
-- transaction rolls back, leaving `game_bite` and its triggers as they were.
-- Dropping the scenario instead would lose what the owner picked.
DROP TRIGGER game_bite_leaves_with_account;

CREATE TABLE game_bite_old (
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
    -- A scenario exactly when the bite was not said first.
    CHECK ((how = 'said-first') = (scenario_id IS NULL))
);

INSERT INTO game_bite_old (subject_id, note_id, scenario_id, how, offer_seq, live_strip, context_line, watching,
                           started_at, clock_offset_ms, mode, catalog_version)
SELECT subject_id, note_id, scenario_id, how, offer_seq, live_strip, context_line, watching,
       started_at, clock_offset_ms, mode, catalog_version FROM game_bite;

DROP TABLE game_bite;
ALTER TABLE game_bite_old RENAME TO game_bite;

CREATE TRIGGER game_bite_is_append_only
BEFORE UPDATE ON game_bite
BEGIN
    SELECT RAISE(ABORT, 'game_bite is append-only: a bite is never rewritten');
END;

CREATE TRIGGER game_bite_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM game_bite WHERE subject_id = OLD.subject_id;
END;
