-- LeetType session logs filed for the Jobs mailbox (paulgsc/some-ui#2027,
-- LTY-MAILBOX 2): which of the owner's LeetType sessions
-- (`leetype_session_event`, 20261020000100) the `mailbox` poller filed in
-- `mailbox_outgoing`, under what number, and how far into the record that
-- file reached. The weekly review reads them there, in the mailbox's
-- `leetype-log` folder (paulgsc/some-ui `apps/mailbox`), since it runs in the
-- cloud and cannot reach home. docs/identity.md invariants 18 and 19.
--
-- A mailbox name holds one body for good, and a session can gain events after
-- its log was filed, so a later copy goes under a name of its own:
-- `log-<session>.json` first, then `log-<session>.r<n>.json`. `n` is decided
-- once, here, as the session's next, as `mailbox_transcript` decides a
-- transcript's.
--
-- `through` is a high-water mark: the largest `leetype_session_event.rowid`
-- the filed copy covered. A pass asks only for the subject's events above the
-- largest `through` it holds (`leetype_session_event_arrival`, below, and
-- `mailbox_leetype_log_through`), so an idle pass reads two index probes and
-- no history, and a session is filed again only once an event of it arrives
-- after its last copy. That relies on `leetype_session_event`'s rowids only
-- ever growing: the table is append-only, nothing here runs `VACUUM` (which
-- may renumber the rowids of a table with no INTEGER PRIMARY KEY), and
-- `auto_vacuum` is off (docs/storage.md). A `VACUUM` of home's database would
-- break this mark; docs/identity.md invariant 18 says so.
--
-- APPEND-ONLY, like the outgoing file it names: no row is updated (the
-- trigger below aborts an `UPDATE`) or deleted but with the account.
--
-- Columns:
--
--   subject_id -- whose session.
--   session_id -- which session: the phone's id for it, as
--                 `leetype_session_event.session_id`.
--   n          -- its number among the session's filed logs, from 1.
--   through    -- the largest `leetype_session_event.rowid` the file covered.
--   filed_at   -- ISO-8601 UTC, when it was filed.
CREATE TABLE mailbox_leetype_log (
    subject_id TEXT    NOT NULL,
    session_id TEXT    NOT NULL,
    n          INTEGER NOT NULL CHECK (n >= 1),
    through    INTEGER NOT NULL,
    filed_at   TEXT    NOT NULL,
    PRIMARY KEY (subject_id, session_id, n)
);

-- The high-water mark: the largest `through` per subject, one probe.
CREATE INDEX mailbox_leetype_log_through ON mailbox_leetype_log (subject_id, through);

-- The events that arrived after the mark: a range of one subject's rowids
-- (every SQLite index carries the rowid after its columns).
CREATE INDEX leetype_session_event_arrival ON leetype_session_event (subject_id);

CREATE TRIGGER mailbox_leetype_log_is_append_only
BEFORE UPDATE ON mailbox_leetype_log
BEGIN
    SELECT RAISE(ABORT, 'mailbox_leetype_log is append-only: a filed log keeps its number');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `mailbox_transcript` does.
CREATE TRIGGER mailbox_leetype_log_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM mailbox_leetype_log WHERE subject_id = OLD.subject_id;
END;
