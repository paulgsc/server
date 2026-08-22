-- The server's copy of `ActivityDefinition`
-- (packages/activity-catalog/src/lib/types.ts, paulgsc/some-ui) -- not a
-- redesign of it, the same relationship `sessions` has to `SessionRecord`
-- (see 20260805000300_create_sessions.up.sql). #269.
--
-- The split between columns and JSON blobs follows the rule that migration
-- argued for: a scalar the policy actually filters on is a real column,
-- because that is what gets indexed; a heterogeneous or opaque structure
-- nothing filters on stays a JSON blob rather than paying for a migration
-- every time the client's shape moves.
--
-- What changed the answer for `activities` is that the policy now filters on
-- more than `sessions` ever did:
--
--   id             -- "honeycomb", "leetype", ... -- the client's ActivityId
--                     union, now open: this table, not the union, is the
--                     source of truth for which activities exist.
--   name, description, icon
--                  -- displayed, and cheap.
--   registry_key   -- the one lookup this schema has to make impossible to
--                     get wrong -- see its UNIQUE index below.
--   layout_tree    -- a closed, four-value vocabulary (`LayoutTreeId`) --
--                     column, with a Rust-side `parse` that refuses an
--                     unrecognised value rather than defaulting it.
--   maturity       -- the recommender filters on this directly: an `early`
--                     construction zone is a poor thing to propose
--                     unprompted. Same refuse-don't-default `parse` as
--                     `layout_tree`.
--   min_duration_ms -- see its own comment below.
--   published_at, version
--                  -- the recommender's newness axis, and #273's
--                     `CurriculumUpdated` producer, both read these.
--   fields, default_config, audio
--                  -- JSON blobs. `fields` is a heterogeneous
--                     `SelectField | DurationField` union with nothing here
--                     that filters on an individual option; `default_config`
--                     is meaningless without `fields` to key it against;
--                     `audio` is disclosure copy nobody queries by. Modelling
--                     any of the three relationally would cost a migration
--                     every time the client's form vocabulary moves, for a
--                     query this table is never asked.
--
-- `toSceneProps` has no column at all -- it is a closure on the client, and
-- CAT4 (#272) is where its server-side replacement gets decided.

CREATE TABLE activities (
    id                TEXT    PRIMARY KEY,
    name              TEXT    NOT NULL,
    description       TEXT    NOT NULL,
    icon              TEXT    NOT NULL,      -- ActivityIconKey; opaque lookup key, not modelled as an enum here
    registry_key      TEXT    NOT NULL,
    layout_tree       TEXT    NOT NULL,      -- study | topik | drama | voice
    maturity          TEXT    NOT NULL,      -- ready | preview | early

    -- Hoisted out of the `fields` JSON blob rather than left inside it. The
    -- recommender's duration rule (#281) is "the field minimum, never the
    -- default", read on every provisioning decision -- re-parsing `fields`
    -- and hunting for the duration entry each time is both slow and a place
    -- to get it wrong silently.
    --
    -- Nullable rather than defaulted: an activity that declares no duration
    -- field at all is a real, expected shape this column has to be able to
    -- say, and #281 has to handle that case rather than have it defaulted
    -- into something that looks chosen. `leetype` was, for a while, the
    -- activity that shape described -- its own entry in catalog.ts argued at
    -- length for having no fields at all -- but some-ui#1130 gave it a
    -- session-duration field, so as of this writing every one of the four
    -- catalogued activities has one and seeding (#270) will not actually
    -- produce a NULL row here. That is a fact about today's catalogue, not a
    -- guarantee this column makes; the NULL case stays real for whatever
    -- activity is next to not need one.
    min_duration_ms   INTEGER,

    published_at      TEXT    NOT NULL,      -- ISO-8601
    version           INTEGER NOT NULL,

    -- JSON TEXT. See the column-vs-blob note above.
    fields            TEXT    NOT NULL,
    default_config    TEXT    NOT NULL,
    audio             TEXT
);

-- The reliable identity: `getActivityByRegistryKey`'s own docstring
-- (activity-catalog/src/lib/catalog.ts) is "robust against a scene having
-- been renamed -- scene_name is not a reliable way back to the activity
-- that produced it, but registry_key always is." A second row claiming the
-- same key would make that promise false, so it is enforced here rather
-- than left to the seed step to get right by convention.
CREATE UNIQUE INDEX idx_activities_registry_key ON activities(registry_key);

-- The recommender's two catalogue-wide questions, one index each -- same
-- shape as `idx_sessions_started_at`/`idx_sessions_completed_at`: each
-- answers exactly one filter and nothing asks for both at once yet.
CREATE INDEX idx_activities_published_at ON activities(published_at);
CREATE INDEX idx_activities_maturity ON activities(maturity);
