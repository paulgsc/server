-- A conversation essay's record (paulgsc/server#1243, HOST16-S; paulgsc/some-ui
-- C1535, #3553, #3573): one log of `some-ui.conversation/2` events per
-- conversation, per subject, whose state is the log replayed through the
-- record's step (paulgsc/some-ui `packages/video-render/src/conversation.ts`,
-- `stepConversation`; home's replay is `conversation_repo::step`). The topic
-- and points the owner gives, each podcast version the agent writes, the
-- owner's flags and withdrawals, home's measure of a version's audio, the
-- green light, the agent's emotion notes and the owner's preview approvals
-- (`approve`) are each one event. Home is the owner's own store, so it is the
-- record.
--
-- Reach inward first: no existing log takes it. `studio_sitting_event` and
-- `leetype_session_event` are per-record logs too, but ordered by their
-- events' own ids and times and holding a sitting's or session's own
-- columns; `app_event` keys an event by id within a family, with no record
-- and no order; `studio_take_move` is ordered by number, as this is, but its
-- columns are a conveyor move's. A conversation's order is its log's
-- position, which the step reads, so generalizing any of them would rebuild
-- an append-only table (a discard site, docs/schema.md) to change what its
-- order means. This table takes the conveyor's shape instead: a number per
-- record, from 1, with no gap.
--
-- It is a record, so it is APPEND-ONLY: an event, once kept, is never
-- updated (the trigger below aborts an UPDATE), and nothing deletes one but
-- the account. A refused event is never written: `conversation_repo::append`
-- replays the log with it first, in the same `BEGIN IMMEDIATE` transaction.
--
-- Columns of `conversation_event`:
--
--   subject_id      -- whose record.
--   conversation_id -- the record's id (some-ui's `showId`: 1 to 64 of
--                      a-z, 0-9 and `-`, not starting with `-`).
--   seq             -- the event's place in the log, from 1, with no gap:
--                      its order, never a clock (`at` is the writer's).
--   event           -- its kind, the schema's ten.
--   at              -- when the writer says it happened, as sent.
--   body            -- the whole event as JSON, as sent; its `event` and
--                      `at` are the columns'.
--   stored_at       -- ISO-8601 UTC, when it reached home.
CREATE TABLE conversation_event (
    subject_id      TEXT    NOT NULL,
    conversation_id TEXT    NOT NULL CHECK (length(conversation_id) BETWEEN 1 AND 64 AND conversation_id NOT GLOB '*[^a-z0-9-]*' AND substr(conversation_id, 1, 1) <> '-'),
    seq             INTEGER NOT NULL CHECK (seq >= 1),
    event           TEXT    NOT NULL CHECK (event IN ('topic', 'point', 'version', 'flag', 'withdraw', 'heard', 'green-light', 'notes', 'offer', 'approve')),
    at              TEXT    NOT NULL,
    body            TEXT    NOT NULL CHECK (json_valid(body) AND json_type(body) = 'object' AND json_extract(body, '$.event') = event AND json_extract(body, '$.at') = at),
    stored_at       TEXT    NOT NULL,
    PRIMARY KEY (subject_id, conversation_id, seq)
);

-- An event is the next in its log: no gap, and none before the first.
CREATE TRIGGER conversation_event_is_next
BEFORE INSERT ON conversation_event
WHEN NEW.seq IS NOT (SELECT COALESCE(MAX(seq), 0) + 1 FROM conversation_event WHERE subject_id = NEW.subject_id AND conversation_id = NEW.conversation_id)
BEGIN
    SELECT RAISE(ABORT, 'conversation_event.seq must be the next in its log');
END;

CREATE TRIGGER conversation_event_is_append_only
BEFORE UPDATE ON conversation_event
BEGIN
    SELECT RAISE(ABORT, 'conversation_event is append-only: keep a new event');
END;

-- Account deletion on every binary that can run against this schema
-- (docs/identity.md invariant 12), as `episode_note` does.
CREATE TRIGGER conversation_event_leaves_with_account
AFTER DELETE ON account
BEGIN
    DELETE FROM conversation_event WHERE subject_id = OLD.subject_id;
END;
