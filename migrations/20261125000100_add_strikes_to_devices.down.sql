-- Reverses 20261125000100_add_strikes_to_devices.up.sql. A device set aside
-- is sent to again once the columns are gone.
ALTER TABLE jobs_notice_device DROP COLUMN last_probe_at;
ALTER TABLE jobs_notice_device DROP COLUMN set_aside_why;
ALTER TABLE jobs_notice_device DROP COLUMN set_aside_at;
ALTER TABLE jobs_notice_device DROP COLUMN strikes;

ALTER TABLE push_subscriptions DROP COLUMN last_probe_at;
ALTER TABLE push_subscriptions DROP COLUMN set_aside_why;
ALTER TABLE push_subscriptions DROP COLUMN set_aside_at;
ALTER TABLE push_subscriptions DROP COLUMN strikes;
