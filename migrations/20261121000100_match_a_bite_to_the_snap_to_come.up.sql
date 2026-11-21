-- A bite said on an offer of the snap to come is matched to that snap
-- (paulgsc/some-ui#3040, promise BM5). Before, every bite was matched to the
-- nearest play its scenario's rule accepts at or before it, so a bite on a
-- third and long, a fourth down or a field goal try, said before the snap,
-- took the play before it or none.
--
-- `method` gains `next`: the first play the scenario's rule accepts that home
-- had not seen when the bite started, its time within the window either side
-- of the bite (`scenario_engine::its_play`, `Said::Before`). Its
-- `gap_seconds` is still the bite's start on home's clock minus the play's
-- time, so it is negative when the play came after the bite, and may be
-- positive on a delayed broadcast; `nearest`'s stays at or above zero. Every
-- other column and CHECK means what 20261113000500 says.
--
-- 20261113000500_create_bite_match has shipped, so it is not edited: SQLite
-- cannot alter a CHECK, so `bite_match` is rebuilt as 20261028000300 rebuilt
-- `game_bite`: a fresh `bite_match_new`, every row copied across unchanged
-- (match_id included, so a bite's newest row stays its newest), the old table
-- dropped and the new one renamed in. Its index and append-only trigger go
-- with the dropped table and are recreated word for word. The account trigger
-- names `bite_match` in its body, so it is dropped first and recreated once
-- the rebuilt table holds the name again. No other index, view or trigger
-- names `bite_match` (every migration grepped).
DROP TRIGGER bite_match_leaves_with_account;

CREATE TABLE bite_match_new (
    match_id       INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_id     TEXT    NOT NULL,
    note_id        TEXT    NOT NULL,
    scenario_id    TEXT,
    play_id        TEXT,
    method         TEXT    NOT NULL CHECK (method IN ('nearest', 'next', 'unmatched', 'not-a-play', 'no-scenario')),
    gap_seconds    INTEGER CHECK (method <> 'nearest' OR gap_seconds >= 0),
    engine_version INTEGER NOT NULL CHECK (engine_version >= 1),
    matched_at     TEXT    NOT NULL,
    -- A play exactly when matched, and a gap exactly with a play.
    CHECK ((method IN ('nearest', 'next')) = (play_id IS NOT NULL)),
    CHECK ((play_id IS NULL) = (gap_seconds IS NULL)),
    -- A scenario unless there was none to match under.
    CHECK ((method = 'no-scenario') = (scenario_id IS NULL))
);

INSERT INTO bite_match_new (match_id, subject_id, note_id, scenario_id, play_id, method, gap_seconds, engine_version, matched_at)
SELECT match_id, subject_id, note_id, scenario_id, play_id, method, gap_seconds, engine_version, matched_at FROM bite_match;

DROP TABLE bite_match;
ALTER TABLE bite_match_new RENAME TO bite_match;

-- A bite's newest match: an indexed range read.
CREATE INDEX bite_match_by_bite ON bite_match (subject_id, note_id, match_id);

CREATE TRIGGER bite_match_is_append_only
BEFORE UPDATE ON bite_match
BEGIN
    SELECT RAISE(ABORT, 'bite_match is append-only: a later match is a new row');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `game_bite` does.
CREATE TRIGGER bite_match_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM bite_match WHERE subject_id = OLD.subject_id;
    DELETE FROM bite_mapping WHERE subject_id = OLD.subject_id;
END;
