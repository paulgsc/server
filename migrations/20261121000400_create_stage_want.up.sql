-- What waits on what across home's shared stages (paulgsc/server#1091,
-- docs/shared-stages.md, "Who waits drives order"): one row per item a
-- waiter is waiting on right now. The edges of the graph `crate::stage`
-- declares (`Stage` the vertex kinds, `Waiter` the edge kinds), so a stage's
-- queue can put first what someone downstream waits on.
--
-- Each waiter's pass replaces all its own rows in one transaction with what it
-- waits on now (`stage_repo::replace`), so a row lives only while its waiter
-- waits, and nothing else writes or cleans it. DERIVED: every row is computed
-- again from the waiter's own record (a cards essay's body) on its next pass,
-- so losing the table loses no one's data.
--
--   waiter      -- the waiter's kind, one of `stage::Waiter`'s names.
--   waiter_key  -- which one: a cards essay's key.
--   stage       -- the stage it waits on, one of `stage::Stage`'s names.
--   subject_id  -- whose item: the takes an essay plays are a subject's.
--   item_key    -- the item, by its stage's own key: a note id.
--
-- No time is kept: what orders the queue is that the wait is there, and a
-- timeline of the owner's waits is nothing anyone reads.
--
-- Names a subject's note, so it leaves with the account (docs/identity.md
-- invariant 12), and holds no words, audio or text of anyone's.
CREATE TABLE stage_want (
    waiter     TEXT NOT NULL CHECK (waiter IN ('cards_essays')),
    waiter_key TEXT NOT NULL,
    stage      TEXT NOT NULL CHECK (stage IN ('transcriber')),
    subject_id TEXT NOT NULL,
    item_key   TEXT NOT NULL,
    PRIMARY KEY (stage, subject_id, item_key, waiter, waiter_key)
);

-- A waiter's own rows, which its pass replaces.
CREATE INDEX stage_want_by_waiter ON stage_want (waiter, waiter_key);

-- A subject's rows, which an account's or a note's deletion removes.
CREATE INDEX stage_want_by_subject ON stage_want (subject_id, item_key);

CREATE TRIGGER stage_want_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM stage_want WHERE subject_id = OLD.subject_id;
END;
