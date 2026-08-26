-- Presence, redesigned as a lease rather than a WebSocket connection count.
--
-- One row per (subject, context) the client says it is currently looking at.
-- `observed_at` is the only mutable column, and it is a timestamp, not a
-- status: there is no `active` flag to flip and nothing here ever expires a
-- row. Freshness is `now - observed_at < ttl`, computed at read time by
-- `nudge::presence::observe` — a six-month-old row is semantically identical
-- to no row at all, so no background job walks this table.
--
-- PRIMARY KEY is (subject_id, context_key), not subject_id alone: a subject
-- can hold more than one fresh context at once, and a stale context must
-- never shadow a fresh one elsewhere. The composite key also serves as this
-- table's only index — the one read path (`for_subject`) filters on
-- `subject_id`, the leftmost column, so no separate index earns its keep.
CREATE TABLE presence_leases (
    subject_id   TEXT NOT NULL,
    context_key  TEXT NOT NULL,
    observed_at  TEXT NOT NULL,   -- ISO-8601 UTC

    PRIMARY KEY (subject_id, context_key)
);
