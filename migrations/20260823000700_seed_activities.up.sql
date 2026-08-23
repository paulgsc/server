-- Seeds `activities` (20260822000600_create_activities.up.sql) with the
-- four entries transcribed from
-- `paulgsc/some-ui@packages/activity-catalog/src/lib/catalog.ts`, field for
-- literal field. #270.
--
-- `leetype` is transcribed as it exists *today*, not as #270's own
-- acceptance criteria described it when last written (2026-08-20):
-- some-ui#1130 (339dd6d, merged 2026-08-21, discovered during #269's work)
-- gave it a real `durationMinutes` field, retiring the "no fields,
-- deliberately" invariant its old comment argued for. That comment is gone
-- from `catalog.ts`; this migration follows the current source, not the
-- issue's stale example -- see the PR description for the full account. As
-- of this seed, every one of the four activities declares a duration field,
-- so no row below has a NULL `min_duration_ms`; the column stays nullable
-- for whichever activity is next to not need one, per the create-table
-- migration's own comment.
--
-- Each `min_duration_ms` is `minMinutes * 60_000` for that activity's
-- `duration` field. This migration does not derive it in SQL -- the parity
-- test (`crates/db/activity/tests/catalog_parity.rs`) re-derives it from
-- `fields` independently, in Rust, and asserts it against the literal
-- inserted here, so a wrong hand-computed number fails loudly instead of
-- being trusted.
--
-- `published_at` is this migration's own authoring date, not a value
-- `catalog.ts` declares -- server-only bookkeeping, same as `version`.
INSERT INTO activities (
    id, name, description, icon, registry_key, layout_tree, maturity,
    min_duration_ms, published_at, version, fields, default_config, audio
) VALUES
(
    'honeycomb',
    'Hangul Honeycomb',
    'Type the falling Hangul characters before they escape the hive.',
    'hexagon',
    'hangul',
    'study',
    'ready',
    300000,
    '2026-08-23T00:00:00Z',
    1,
    '[{"kind":"select","key":"mode","label":"Mode","options":[{"value":"completion","label":"Completion (clear the board)"},{"value":"endless","label":"Endless (survive as long as you can)"},{"value":"vocabulary","label":"Vocabulary (master the word list)"},{"value":"vocabulary-endless","label":"Vocabulary Endless (words, no end)"}],"defaultValue":"completion"},{"kind":"select","key":"difficulty","label":"Difficulty","options":[{"value":"relaxed","label":"Relaxed - more time per letter"},{"value":"standard","label":"Standard"},{"value":"challenging","label":"Challenging - fast-paced"}],"defaultValue":"standard"},{"kind":"duration","key":"durationMinutes","label":"Session length","minMinutes":5,"maxMinutes":30,"stepMinutes":5,"defaultMinutes":10}]',
    '{"mode":"completion","difficulty":"standard","durationMinutes":10}',
    '{"channels":["effects"],"blurb":"Uses game sound effects"}'
),
(
    'topik',
    'TOPIK Study',
    'Work through Korean study material with a guided chat and quiz.',
    'book-open',
    'topik',
    'topik',
    'preview',
    600000,
    '2026-08-23T00:00:00Z',
    1,
    '[{"kind":"select","key":"level","label":"Level","options":[{"value":"beginner","label":"Beginner"},{"value":"intermediate","label":"Intermediate"},{"value":"advanced","label":"Advanced"}],"defaultValue":"beginner"},{"kind":"duration","key":"durationMinutes","label":"Session length","minMinutes":10,"maxMinutes":45,"stepMinutes":5,"defaultMinutes":15}]',
    '{"level":"beginner","durationMinutes":15}',
    '{"channels":["speech"],"blurb":"Uses Korean pronunciation"}'
),
(
    'interview',
    'Interview Prep',
    'Practice answering mock interview questions on a timer.',
    'mic',
    'interview',
    'topik',
    'early',
    600000,
    '2026-08-23T00:00:00Z',
    1,
    '[{"kind":"select","key":"level","label":"Level","options":[{"value":"junior","label":"Junior"},{"value":"mid","label":"Mid"},{"value":"senior","label":"Senior"}],"defaultValue":"mid"},{"kind":"select","key":"category","label":"Category","options":[{"value":"technical","label":"Technical"},{"value":"behavioral","label":"Behavioral"},{"value":"system-design","label":"System Design"}],"defaultValue":"technical"},{"kind":"duration","key":"durationMinutes","label":"Session length","minMinutes":10,"maxMinutes":45,"stepMinutes":5,"defaultMinutes":20}]',
    '{"level":"mid","category":"technical","durationMinutes":20}',
    '{"channels":["speech"],"blurb":"Reads questions aloud"}'
),
(
    'leetype',
    'LeetType',
    'Prove one competency at a time by typing the smallest code that shows it.',
    'keyboard',
    'leetype',
    'study',
    'ready',
    300000,
    '2026-08-23T00:00:00Z',
    1,
    '[{"kind":"duration","key":"durationMinutes","label":"Session length","minMinutes":5,"maxMinutes":60,"stepMinutes":5,"defaultMinutes":10}]',
    '{"durationMinutes":10}',
    NULL
);
