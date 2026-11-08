-- Admin settings (paulgsc/server#501, ADMIN1; the first lever is Rust Rounds
-- recording, paulgsc/some-ui#2357): the levers the operator turns while home
-- runs, kept as revisions.
--
-- A setting is the deployment's, not a person's: it is read by home's own
-- background work (the Rust Rounds recorder reads `rust-rounds` once a pass)
-- and written only by the operator, through `PUT /api/v1/admin/settings/:ns`.
-- Each namespace's schema and defaults live in code (`file_host::settings`),
-- so this table holds only what the operator has set, and a namespace with no
-- row reads as its default.
--
-- APPEND-ONLY. A change adds a revision; no row is ever updated or deleted
-- (the triggers below abort both), so every value a setting once held, who
-- set it and when, stays readable (`GET /api/v1/admin/settings/:ns/history`).
--
-- Columns:
--
--   namespace -- which setting: `rust-rounds`, and later others, each named
--                in `file_host::settings::NAMESPACES`.
--   revision  -- 1 for the first value set, then 2, 3 … per change. A write
--                of the value already standing adds nothing.
--   body      -- the value as JSON, re-serialised after its schema accepted it.
--   set_by    -- who set it, as a role, never a subject: `owner` (the only
--                account, docs/identity.md "The owner's account") or
--                `operator` (a subject `OPERATOR_SUBJECTS` lists). No subject
--                id is kept, so the history outlives any account without
--                naming one (docs/identity.md invariant 37).
--   set_at    -- ISO-8601 UTC, when this revision was written.
CREATE TABLE admin_setting (
    namespace TEXT    NOT NULL,
    revision  INTEGER NOT NULL CHECK (revision >= 1),
    body      TEXT    NOT NULL,
    set_by    TEXT    NOT NULL CHECK (set_by IN ('owner', 'operator')),
    set_at    TEXT    NOT NULL,
    PRIMARY KEY (namespace, revision)
);

CREATE TRIGGER admin_setting_is_append_only
BEFORE UPDATE ON admin_setting
BEGIN
    SELECT RAISE(ABORT, 'admin_setting is append-only: write a new revision');
END;

CREATE TRIGGER admin_setting_is_never_deleted
BEFORE DELETE ON admin_setting
BEGIN
    SELECT RAISE(ABORT, 'admin_setting keeps every revision');
END;
