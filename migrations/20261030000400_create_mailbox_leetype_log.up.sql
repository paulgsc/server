-- LeetType session logs filed for the Jobs mailbox (paulgsc/some-ui#2027,
-- LTY-MAILBOX 2): what the `mailbox` poller decided for each of the owner's
-- LeetType sessions (`leetype_session_event`, 20261020000100) as its events
-- arrived: filed in `mailbox_outgoing` under a number, or refused, and how far
-- into the record that decision reached. The weekly review reads the filed
-- logs in the mailbox's `leetype-log` folder (paulgsc/some-ui `apps/mailbox`),
-- since it runs in the cloud and cannot reach home. docs/identity.md
-- invariants 18 and 19.
--
-- A mailbox name holds one body for good, and a session can gain events after
-- its log was filed, so a later copy goes under a name of its own:
-- `log-<session>.json` first, then `log-<session>.r<n>.json`. `n` is decided
-- once, here, as the session's next, as `mailbox_transcript` decides a
-- transcript's.
--
-- A session the poller cannot file (an event that does not read back, no
-- `started` event, a body over the mailbox's 1 MiB) gets a row too, with
-- `refused` saying why and no `n`, and nothing in `mailbox_outgoing`. Only
-- the mailbox copy is skipped: the session's events stay in
-- `leetype_session_event`, untouched, and an event of it that arrives later
-- makes it due again, which is when filing it could succeed.
--
-- `through` is a high-water mark: the largest `leetype_session_event.rowid`
-- the decision covered, filed or refused. A pass asks only for the subject's
-- events above the largest `through` it holds (`leetype_session_event_arrival`,
-- below, and this table's primary key), so an idle pass reads two index
-- probes and no history, and no number of refused sessions holds back the
-- ones after them. That relies on one property of `leetype_session_event`: a
-- new event's rowid is above every rowid its subject already holds. Appends
-- keep it, and account deletion keeps it because it takes the subject's log
-- rows too; only `VACUUM` breaks it (it may renumber the rowids of a table
-- with no INTEGER PRIMARY KEY), and nothing here runs one, with `auto_vacuum`
-- off (docs/storage.md). docs/identity.md invariant 18 says so.
--
-- APPEND-ONLY, like the outgoing file it names: no row is updated (the
-- trigger below aborts an `UPDATE`) or deleted but with the account.
--
-- Columns:
--
--   subject_id -- whose session.
--   session_id -- which session: the phone's id for it, as
--                 `leetype_session_event.session_id`.
--   through    -- the largest `leetype_session_event.rowid` the decision
--                 covered; each decision's is above the one before it.
--   n          -- for a filed log, its number among the session's filed
--                 logs, from 1; NULL for a refusal.
--   refused    -- for a refusal, why: `unreadable`, `no_file` or `too_big`;
--                 NULL for a filed log.
--   decided_at -- ISO-8601 UTC, when it was filed or refused.
CREATE TABLE mailbox_leetype_log (
    subject_id TEXT    NOT NULL,
    session_id TEXT    NOT NULL,
    through    INTEGER NOT NULL,
    n          INTEGER CHECK (n >= 1),
    refused    TEXT    CHECK (refused IN ('unreadable', 'no_file', 'too_big')),
    decided_at TEXT    NOT NULL,
    PRIMARY KEY (subject_id, through),
    UNIQUE (subject_id, session_id, n),
    CHECK ((n IS NULL) <> (refused IS NULL))
);

-- The events that arrived after the mark: a range of one subject's rowids
-- (every SQLite index carries the rowid after its columns).
CREATE INDEX leetype_session_event_arrival ON leetype_session_event (subject_id);

CREATE TRIGGER mailbox_leetype_log_is_append_only
BEFORE UPDATE ON mailbox_leetype_log
BEGIN
    SELECT RAISE(ABORT, 'mailbox_leetype_log is append-only: a decision is never rewritten');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `mailbox_transcript` does.
CREATE TRIGGER mailbox_leetype_log_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM mailbox_leetype_log WHERE subject_id = OLD.subject_id;
END;
