-- LeetType session logs filed for the Jobs mailbox (paulgsc/some-ui#2027,
-- LTY-MAILBOX 2): which of the owner's finished LeetType sessions
-- (`leetype_session_event`, 20261020000100) the `mailbox` poller filed in
-- `mailbox_outgoing`, under what number, and how many events that file held.
-- The weekly review reads them there, in the mailbox's `leetype-log` folder
-- (paulgsc/some-ui `apps/mailbox`), since it runs in the cloud and cannot
-- reach home. docs/identity.md invariants 18 and 19.
--
-- A mailbox name holds one body for good, and a session can gain events after
-- its log was filed (the phone sends a session home once it is done with, and
-- may send more of it later), so a later copy goes under a name of its own:
-- `log-<session>.json` first, then `log-<session>.r<n>.json`. `n` is decided
-- once, here, as the session's next, as `mailbox_transcript` decides a
-- transcript's; `events` is how many events that file held, so the poller
-- files a new copy only once home holds more of the session than the last
-- one did. A file holds every event home holds of the session, in the order
-- `GET /leetype/sessions/:session/events` serves them, and nothing else: no
-- soundbite, since `leetype_session_event` holds none.
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
--   events     -- how many of the session's events that file held.
--   filed_at   -- ISO-8601 UTC, when it was filed.
CREATE TABLE mailbox_leetype_log (
    subject_id TEXT    NOT NULL,
    session_id TEXT    NOT NULL,
    n          INTEGER NOT NULL CHECK (n >= 1),
    events     INTEGER NOT NULL CHECK (events >= 1),
    filed_at   TEXT    NOT NULL,
    PRIMARY KEY (subject_id, session_id, n)
);

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
