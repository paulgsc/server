-- What home keeps, by table, once a day (paulgsc/server#542, STORAGE1).
-- `file_host`'s `storage` sampler is the one writer; `storage_repo` is the
-- one module that writes this table. `docs/storage.md` records the design.
--
-- Not subject-scoped: bytes per table, index and file, nobody's record. No
-- column names a person, and nothing here is read per subject.
--
-- APPEND-ONLY. A day's row set is written once, in one transaction, and the
-- triggers abort every `UPDATE` and `DELETE`, and an insert over a row already
-- held: `INSERT OR REPLACE` deletes the old row without firing a `DELETE`
-- trigger (`recursive_triggers` is off). The history is what a growth rate is
-- read from, and Prometheus keeps only 200 hours of it.
--
-- One row set per UTC day. Each set holds:
--
--   * one row per table and per index, `kind` = 'table' or 'index', its
--     bytes read from `dbstat` (every page of that b-tree, overflow pages
--     included, free space inside them included);
--   * `kind` = 'database': the whole file, `page_count * page_size`;
--   * `kind` = 'wal': the `-wal` file's length at that moment, 0 if absent;
--   * `kind` = 'disk_free': the bytes free to the server on the database's
--     filesystem (`statvfs`: blocks available to an unprivileged writer).
--
-- For the last three, `name` repeats the kind. Columns:
--
--   day        -- the UTC date the set belongs to (YYYY-MM-DD).
--   sampled_at -- ISO-8601 UTC, when the set was taken; its date is `day`.
--   kind       -- 'table', 'index', 'database', 'wal' or 'disk_free'.
--   name       -- the table's or index's name; the kind for the others.
--   bytes      -- the size in bytes.
CREATE TABLE storage_sample (
    day        TEXT    NOT NULL CHECK (date(day) IS day),
    sampled_at TEXT    NOT NULL CHECK (substr(sampled_at, 1, 10) = day),
    kind       TEXT    NOT NULL CHECK (kind IN ('table', 'index', 'database', 'wal', 'disk_free')),
    name       TEXT    NOT NULL CHECK (length(name) > 0),
    bytes      INTEGER NOT NULL CHECK (bytes >= 0),
    CHECK (kind IN ('table', 'index') OR name = kind),
    PRIMARY KEY (day, kind, name)
);

CREATE TRIGGER storage_sample_no_replace
BEFORE INSERT ON storage_sample
WHEN EXISTS (SELECT 1 FROM storage_sample WHERE day = NEW.day AND kind = NEW.kind AND name = NEW.name)
BEGIN
    SELECT RAISE(ABORT, 'storage_sample is append-only: a day''s sample is never replaced');
END;

CREATE TRIGGER storage_sample_no_update
BEFORE UPDATE ON storage_sample
BEGIN
    SELECT RAISE(ABORT, 'storage_sample is append-only: a day''s sample is never rewritten');
END;

CREATE TRIGGER storage_sample_no_delete
BEFORE DELETE ON storage_sample
BEGIN
    SELECT RAISE(ABORT, 'storage_sample is append-only: a day''s sample is never deleted');
END;
