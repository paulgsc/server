-- #325 (LTY-SRV1): LeetType's round corpus, server-owned, the way #274 moved
-- TOPIK lessons into `curriculum`.
--
-- WHAT A ROW IS. A **round** (paulgsc/server#324's amendment; `RoundSchema` in
-- `paulgsc/some-ui@packages/ui/leetype`), not the M20 step `Exercise` this
-- issue was first drafted against: a complete program `A`
-- (`algorithm.{language, entryPoint, inputAlphabet, source}`), a constraint
-- diff `C` (`constraintDiff.{before, after}`), an operation budget `B`, a cost
-- graph `G`, and the option set `D` (`diffOptions[]`), each member carrying a
-- hunk, its own cost graph, rescue candidates, and `μ` -- the `CW-P`
-- proposition in the canon's fixed, numbered register that the member
-- witnesses -- with exactly one member admissible.
--
-- THE SHAPE DECISION: an opaque, verbatim body per round, plus a queryable
-- `μ` edge table. Not one or the other.
--
-- * The body is a blob for the reason `20260805000300_create_sessions.up.sql`
--   gave for `scenes`: programs, cost graphs, hunks and rescue candidates are
--   nested structures this server never reasons about, and whose shape moves
--   with the client. Modelling them in SQL would buy no query anybody asks and
--   cost a migration every time `RoundSchema` grows a field. The client owns
--   what a round *is* and checks it with its own schema and corpus lint.
-- * `μ` is the exception, because it is the one access pattern that is not
--   "hand the round back whole". It is a many-to-one edge into a register that
--   is fixed and corpus-independent (`CW-P1` ... ), and two client consumers
--   (the learner ledger and the round sampler) want "which rounds witness
--   CW-P5" -- a question a blob answers only by reading every round. So each
--   member of `D` is also a row in `leetype_round_witness`: the round, the
--   member's position, its proposition, and whether it is the admissible one.
--   The edge is derived from the body on every write (`leetype_round_repo`)
--   and rewritten wholesale, in the same transaction, whenever the body's
--   bytes change; it is never written on its own, so it cannot disagree with
--   the body it came from.
--
-- WHAT THE SERVER CHECKS, AND WHAT STAYS CLIENT-SIDE. On the way in the
-- server reads exactly `id`, `algorithm.language`, and each member's
-- `propositionId` and `admissible`: enough to key the row, fill the edge
-- table, and refuse a round whose `D` does not have exactly one admissible
-- member (`leetype_round_repo::parse_round`). #324's whole-corpus lints --
-- citation resolution against the canon's register, distractor coverage, and
-- `0 < |C| <= |D|` -- stay the client's corpus lint, run before export. The
-- server checks a `CW-P` identifier's *form*, not that the register defines
-- it: the register is the canon's, and a copy of it here would drift.
--
-- NOT HERE, ON PURPOSE: an execution transcript per round. Producing one means
-- running `A`, an execution route (paulgsc/some-ui#1226) this server does not
-- have; it gets its own column or table when that lands, not a guessed shape
-- now. And no `curriculum_publication` involvement: a new round is not
-- announced by the study nudge. Whether it should be is left for later.
--
-- Columns, each for a consumer:
--
--   id           -- `Round.id`, the key the client fetches by; a URL path
--                   segment, held to `curriculum_repo::is_plain_key`'s rule.
--   language     -- `algorithm.language`; the client's schema admits only
--                   `rust` today, and so does this CHECK. A second language
--                   is a migration that says what else changes with it.
--   published_at -- ISO-8601 UTC; set when a round first appears or its bytes
--                   change, and not when a re-import finds identical bytes.
--   version      -- bumped on every content change, like `curriculum.version`.
--   content_hash -- lowercase hex SHA-256 over the body's exact bytes; the
--                   importer's idempotence and the route's ETag.
--   retired_at   -- ISO-8601 UTC when the operator retired it, or NULL while
--                   listed. Retired rounds leave the manifest but stay
--                   readable by id, exactly as `curriculum.retired_at` does.
--   body         -- the round's JSON, verbatim; owned by `@some-ui/leetype`.
CREATE TABLE leetype_round (
    id           TEXT    PRIMARY KEY,
    language     TEXT    NOT NULL CHECK (language = 'rust'),
    published_at TEXT    NOT NULL,   -- ISO-8601 UTC
    version      INTEGER NOT NULL,
    content_hash TEXT    NOT NULL,   -- hex SHA-256 of `body`'s bytes
    retired_at   TEXT,               -- ISO-8601 UTC, or NULL while listed
    body         TEXT    NOT NULL    -- the round, verbatim; owned by @some-ui/leetype
);

-- The listed set, by id: the manifest's `ORDER BY id LIMIT ?` and its count
-- read it without walking retired rows, which only accumulate.
CREATE INDEX idx_leetype_round_listed ON leetype_round(id) WHERE retired_at IS NULL;

-- `μ`, one row per member of a round's option set `D`.
--
--   member_index   -- the member's position in `diffOptions`, from 0.
--   proposition_id -- `member.propositionId`, `CW-P<n>`.
--   admissible     -- `member.admissible`; exactly one 1 per round.
CREATE TABLE leetype_round_witness (
    round_id       TEXT    NOT NULL REFERENCES leetype_round(id) ON DELETE CASCADE,
    member_index   INTEGER NOT NULL,
    proposition_id TEXT    NOT NULL,
    admissible     INTEGER NOT NULL CHECK (admissible IN (0, 1)),
    PRIMARY KEY (round_id, member_index)
);

-- "Which rounds witness CW-P5": an index range read by proposition, the round
-- id carried in the index so the join to the listed rounds needs no table
-- lookup to find it.
CREATE INDEX idx_leetype_round_witness_proposition ON leetype_round_witness(proposition_id, round_id);
