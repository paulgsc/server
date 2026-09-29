-- #387: the learner shelf. A learner may choose to keep some of the content
-- they generated themselves (a TOPIK lesson; a LeetType round, per
-- paulgsc/some-ui#1598) on the server, to replay it on another device. One
-- table serves both activities, keyed by activity so their keys never collide.
--
-- It is a shelf, not a library, and its shape is canon Rem. 7.3's five points
-- as #387 states them:
--
-- 1. OPT-IN, PER ITEM. A row is written only by `PUT /shelf/:activity/:key`,
--    which the client sends when the learner asks to keep that item. Nothing
--    syncs in the background: no other route, the waker, and no importer or
--    binary writes this table (`learner_shelf_repo`'s module docs carry the
--    grep-level claim).
-- 2. STORED AGAINST THE SUBJECT. Keyed by `subject_id` (the `SubjectId` a
--    passkey session resolves to), and listed in
--    `subject::SUBJECT_SCOPED_TABLES`, so `DELETE /auth/account` removes it
--    and the privacy schema test covers it. No columns beyond what storing
--    and listing an item needs.
-- 3. CAPPED PER SUBJECT. At most `learner_shelf_repo::SHELF_CAP` (20) items
--    per subject per activity, matching the client's `MAX_LOCAL_LESSONS`, and
--    at most `SHELF_BODY_CEILING` (256 KiB) per body. Over the cap a new key
--    is refused (`409`), never evicted: the client decides what to delete.
--    The cap is enforced in the insert statement itself, so no interleaving
--    of writes can exceed it.
-- 4. CONTENT ONLY. A row holds the body verbatim, its key, its content hash
--    and when it was saved. Never a survey, a flag, an outcome, a title or
--    anything derived from play; the server checks only that the body is a
--    JSON object or array within the ceiling ("the server never parses a
--    lesson", canon Def. 8.3).
-- 5. LOSING IT IS ACCEPTABLE. A convenience copy: no backup, no retention
--    promise, no soft delete. `DELETE` removes the row.
--
-- HARD CONSTRAINTS (#387):
--
-- * Never in `curriculum`, never in `curriculum_publication`, never in
--   `leetype_round`. Those are the corpus everyone is served, and #277's
--   detection would announce a learner's item (and its timing) to every
--   subject who played the activity. This is a separate table no detection
--   reads, and a shelf write touches no other table (pinned by
--   `handlers::shelf`'s tests).
-- * Not readable across subjects. Every read and write is `WHERE subject_id
--   = ?` with the request's own subject; there is no shared or public
--   listing. Another subject's key is a `404`, the same answer as a key
--   nobody holds.
-- * Blocked on auth: passkey auth (#395) landed first, so every shelf route
--   takes a `SubjectId` and answers `401` without a session. There is no
--   fallback subject.
--
-- Columns:
--
--   subject_id   -- whose shelf.
--   activity_id  -- `topik` or `leetype`; a new activity is a migration that
--                   says why its generated content belongs here.
--   key          -- the client's key for the item; a URL path segment, held
--                   to `curriculum_repo::is_plain_key`'s rule.
--   content_hash -- lowercase hex SHA-256 over the body's exact bytes
--                   (`curriculum_repo::content_hash`); what makes a re-`PUT`
--                   of the same bytes a no-op, and what the client compares.
--   saved_at     -- ISO-8601 UTC, when these bytes were kept. Moves only when
--                   the bytes change. This is the one instant stored, and it
--                   says when the learner last kept something: the price of
--                   the manifest's order, recorded in docs/identity.md,
--                   "What is still exposed".
--   body         -- the item's JSON, verbatim; owned by the client.
--
-- No index beyond the primary key: every query is by (subject_id,
-- activity_id[, key]), which the key's prefix serves, and a listing sorts at
-- most `SHELF_CAP` rows.
CREATE TABLE learner_shelf (
    subject_id   TEXT NOT NULL,
    activity_id  TEXT NOT NULL CHECK (activity_id IN ('topik', 'leetype')),
    key          TEXT NOT NULL,
    content_hash TEXT NOT NULL,   -- hex SHA-256 of `body`'s bytes
    saved_at     TEXT NOT NULL,   -- ISO-8601 UTC
    body         TEXT NOT NULL,   -- verbatim; owned by the client
    PRIMARY KEY (subject_id, activity_id, key)
);
