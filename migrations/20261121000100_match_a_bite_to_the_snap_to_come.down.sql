-- Reverses 20261121000100_match_a_bite_to_the_snap_to_come.up.sql: rebuilds
-- `bite_match` back to the CHECKs 20261113000500 declared, the same way.
--
-- **It refuses rather than discards,** as 20261028000300's down does: every
-- row is copied into a table whose CHECK refuses `next`, so once a bite has
-- been matched to the snap to come this fails on that row and the migration's
-- transaction leaves the schema as it was. Going back past this means
-- deciding what those matches become first.
DROP TRIGGER bite_match_leaves_with_account;

CREATE TABLE bite_match_old (
    match_id       INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_id     TEXT    NOT NULL,
    note_id        TEXT    NOT NULL,
    scenario_id    TEXT,
    play_id        TEXT,
    method         TEXT    NOT NULL CHECK (method IN ('nearest', 'unmatched', 'not-a-play', 'no-scenario')),
    gap_seconds    INTEGER CHECK (gap_seconds >= 0),
    engine_version INTEGER NOT NULL CHECK (engine_version >= 1),
    matched_at     TEXT    NOT NULL,
    CHECK ((method = 'nearest') = (play_id IS NOT NULL)),
    CHECK ((play_id IS NULL) = (gap_seconds IS NULL)),
    CHECK ((method = 'no-scenario') = (scenario_id IS NULL))
);

INSERT INTO bite_match_old (match_id, subject_id, note_id, scenario_id, play_id, method, gap_seconds, engine_version, matched_at)
SELECT match_id, subject_id, note_id, scenario_id, play_id, method, gap_seconds, engine_version, matched_at FROM bite_match;

-- Carry AUTOINCREMENT's high-water mark across: copying every row keeps
-- each match_id, but a higher id already handed out (to rows an account's
-- deletion removed since) lives only in `sqlite_sequence`, and without it
-- the rebuilt table could hand that id out again.
INSERT INTO sqlite_sequence (name, seq)
SELECT 'bite_match_old', seq FROM sqlite_sequence
WHERE name = 'bite_match' AND NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = 'bite_match_old');
UPDATE sqlite_sequence
SET seq = MAX(seq, (SELECT seq FROM sqlite_sequence WHERE name = 'bite_match'))
WHERE name = 'bite_match_old' AND EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = 'bite_match');

DROP TABLE bite_match;
ALTER TABLE bite_match_old RENAME TO bite_match;

CREATE INDEX bite_match_by_bite ON bite_match (subject_id, note_id, match_id);

CREATE TRIGGER bite_match_is_append_only
BEFORE UPDATE ON bite_match
BEGIN
    SELECT RAISE(ABORT, 'bite_match is append-only: a later match is a new row');
END;

CREATE TRIGGER bite_match_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM bite_match WHERE subject_id = OLD.subject_id;
    DELETE FROM bite_mapping WHERE subject_id = OLD.subject_id;
END;
