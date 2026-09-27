-- The operator's weekly batch (canon Cor. 8.3, `paulgsc/some-ui`): learners
-- choose from a small set the operator curates, not from every lesson ever
-- written. `import-curriculum` never deletes, and nothing should -- a
-- learner's resume point or survey can name a lesson long after it stops
-- being offered. So a lesson leaves the batch by being **retired**: unlisted,
-- not deleted.
--
--   retired_at -- ISO-8601 UTC when the operator retired it, or NULL while it
--                 is listed. The manifest (`GET /curriculum/manifest`) serves
--                 exactly the NULL rows, so the listed set *is* the batch. The
--                 lesson itself stays readable by key (`GET /curriculum/:key`).
--
-- WHAT RETIRING DOES TO `curriculum_publication`: nothing, and restoring
-- doesn't either. A publication is a *listed* `(key, version)` the log has
-- not seen (`PublicationRepository::detect_curriculum_publications`). Retiring
-- and restoring move no version, so neither one appends a row; a lesson whose
-- bytes changed while it was retired is announced when it is restored, because
-- that version is new to everyone. And a retired lesson applies to nobody at
-- catch-up (`relevant_since`), as a deleted one did.
--
-- Every existing row stays listed: deploying this changes no manifest.
ALTER TABLE curriculum ADD COLUMN retired_at TEXT;

-- The listed set, by key: the manifest's `ORDER BY key LIMIT ?` and the
-- waker's detection pass read it without walking retired rows, which only
-- ever accumulate.
CREATE INDEX idx_curriculum_listed ON curriculum(key) WHERE retired_at IS NULL;
