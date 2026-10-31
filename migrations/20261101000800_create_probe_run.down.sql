-- Reverses 20261101000800_create_probe_run.up.sql. The runs and results go
-- with their tables; the probe notes they put through stay in `voice_note`.
DROP TRIGGER probe_result_is_never_deleted;
DROP TRIGGER probe_result_is_append_only;
DROP TRIGGER probe_run_is_never_deleted;
DROP TRIGGER probe_run_is_append_only;
DROP TABLE probe_result;
DROP INDEX probe_run_by_started_at;
DROP TABLE probe_run;
