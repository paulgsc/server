-- #274 (CUR1): a lesson is a row with a timestamp, not the mtime of a file on a
-- bind mount that differs per developer laptop.
--
-- The corpus today is `paulgsc/some-ui@packages/some-content/public/topiks/`:
-- a `manifest.json` plus one `<key>.json` per lesson, fetched by
-- `apps/www/src/lib/topik-content/index.ts`. The same column-vs-blob rule the
-- sessions and activities migrations use applies: columns for what something
-- filters, sorts, or lists by; a blob for what is only ever handed back whole.
--
-- THE MANIFEST-FACING COLUMNS ARE TRANSCRIBED, NOT DESIGNED. They are
-- `TopikMetadataSchema` field for field
-- (`packages/ui/topik/src/lib/topik/entity/topik-metadata.ts`, `paulgsc/some-ui`):
--
--     key: string                 -> key            (TEXT PRIMARY KEY)
--     displayName: string         -> display_name
--     description: string         -> description
--     batchCount: number          -> batch_count
--     totalQuestions: number      -> total_questions
--     totalMessages: number       -> total_messages
--     difficulty?: beginner | intermediate | advanced
--                                 -> level (nullable, because it is optional there)
--     tags?: string[]             -> tags (JSON array TEXT, nullable)
--
-- wrapped by `TopikManifestSchema = { version: string, topiks: TopikMetadata[] }`
-- -- which is `apps/www`'s own `manifestShapeSchema`, `{ version, topiks: unknown[] }`,
-- with the entries typed. CUR3 (#276) assembles exactly that shape from these
-- rows, so the client needs no change beyond its base path.
--
-- Columns this server adds, each for a consumer:
--
--   activity_id  -- which catalogue activity this material is for (`topik`
--                   today; #269's `activities.id`, no REFERENCES as elsewhere).
--                   The join the recommender's newness axis and #277's audience
--                   rule need.
--   published_at -- ISO-8601 UTC; indexed. The entire freshness story: set when
--                   a lesson first appears or its content changes, and *not*
--                   when a re-import finds identical bytes (#275).
--   version      -- bumped on every content change, like `activities.version`.
--   content_hash -- lowercase hex SHA-256 over the lesson file's exact bytes,
--                   as read from disk -- `curriculum_repo::content_hash`. One
--                   notion of "did this change", with three consumers: #275's
--                   idempotent re-import, #276's ETag, and #277's
--                   `CurriculumUpdated`. Over the file, not the manifest
--                   entry: renaming a lesson's `displayName` is not new
--                   material, and must not drain anyone's freshness.
--
-- THE BLOB: `body` is the lesson file's bytes, verbatim. `@some-ui/topik` owns
-- what a lesson *is* and validates it with its own schema on the way in; this
-- server never parses it, and must not acquire opinions about quiz items --
-- re-modelling it here would cost a migration every time the applet's content
-- shape moves, the cost the sessions migration refused to pay for `scenes`.
--
-- Media stays with `gdrive`/`audio_files`; per-subject progress through a lesson
-- is #258's grain, not a column here.
CREATE TABLE curriculum (
    key              TEXT    PRIMARY KEY,
    activity_id      TEXT    NOT NULL,
    level            TEXT    CHECK (level IS NULL OR level IN ('beginner', 'intermediate', 'advanced')),
    display_name     TEXT    NOT NULL,
    description      TEXT    NOT NULL,
    batch_count      INTEGER NOT NULL,
    total_questions  INTEGER NOT NULL,
    total_messages   INTEGER NOT NULL,
    tags             TEXT,               -- JSON array of strings, or NULL when absent

    published_at     TEXT    NOT NULL,   -- ISO-8601 UTC
    version          INTEGER NOT NULL,
    content_hash     TEXT    NOT NULL,   -- hex SHA-256 of `body`'s bytes

    body             TEXT    NOT NULL    -- the lesson file, verbatim; owned by @some-ui/topik
);

-- "What is new since then" -- freshness, and #277.
CREATE INDEX idx_curriculum_published_at ON curriculum(published_at);

-- "Material for activity A at level L".
CREATE INDEX idx_curriculum_activity_level ON curriculum(activity_id, level);
