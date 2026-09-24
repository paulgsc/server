-- #275 (CUR2): a row in `curriculum_publication` that records material as
-- *seen* without announcing it.
--
-- The first import of an existing lesson corpus is what everyone has been
-- studying all along. Its rows must be in the log -- so detection never mistakes
-- them for new -- but must not move the epoch, or every subject would fall
-- behind it and be drained for nothing. The alternative, advancing every
-- subject's watermark past them, is a write per subject: exactly what #273's
-- scaling invariants rule out (`docs/study-nudge.md`).
--
-- So the epoch is the newest row with `baseline = 0`: `PublicationRepository::
-- newest`, and the stamp `EngagementRepository` puts on a new gate row. Rows
-- already in the log -- #273's catalogue baseline -- stay 0: every gate row was
-- stamped with their maximum when that migration ran, so they are the epoch
-- everyone is already at.
ALTER TABLE curriculum_publication ADD COLUMN baseline INTEGER NOT NULL DEFAULT 0 CHECK (baseline IN (0, 1));

-- The epoch lookups -- `newest()` on every waker pass, and the stamp on every
-- new gate row -- read the newest `baseline = 0` row. A corpus's first import
-- appends up to `MANIFEST_CEILING` baseline rows *after* the last real
-- publication; without this they would be walked on every pass. With it, the
-- lookup is one seek from the top of this index (a real
-- `chatgpt-codex-connector` finding on #365).
CREATE INDEX idx_curriculum_publication_epoch ON curriculum_publication(id) WHERE baseline = 0;
