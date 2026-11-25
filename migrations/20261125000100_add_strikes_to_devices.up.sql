-- A device that keeps failing on its own is set aside, not sent to every
-- time (server#1180, server#1222, epic #1174). The owner's words on #1180:
-- "stop sending to a phone after a few timeouts"; the device is set aside,
-- not deleted, "marked as no longer sent to, with the time and why, so the
-- record of it stays, and a delivery it accepts later (or the phone
-- registering again) brings it back". The count is the owner's, 3, kept in
-- code (`file_host::strikes::SET_ASIDE_AFTER`); which failures strike is the
-- coordinator's call C1513, per sender, in the same module.
--
-- Columns on each device table, not a shared table: a shared one would hold
-- an endpoint or a token a second time (docs/identity.md, C1513). Both
-- tables are standing requests a device makes and withdraws, not records,
-- so their lifecycles are as they were; ADD COLUMN rebuilds nothing.
--
-- On `push_subscriptions` (the study nudge's browsers) and
-- `jobs_notice_device` (the Jobs notice's phones):
--
--   strikes       -- failures in a row that were the device's own; a
--                    delivery it accepts, or registering again, sets it to 0.
--                    `push_subscriptions.failure_count` counts every
--                    non-acceptance, home's own included, so it is not this.
--   set_aside_at  -- ISO-8601 UTC, when `strikes` reached the count; NULL
--                    while the device is sent to.
--   set_aside_why -- a fixed label (`timeouts`, `no_answer`,
--                    `sender_mismatch`, `refused`), never the service's
--                    words, which may name the device.
--   last_probe_at -- ISO-8601 UTC, when a set-aside device was last included
--                    in a real send to see whether it takes one again: at
--                    most once a day, and only when a send happens anyway.
ALTER TABLE push_subscriptions ADD COLUMN strikes INTEGER NOT NULL DEFAULT 0 CHECK (strikes >= 0);
ALTER TABLE push_subscriptions ADD COLUMN set_aside_at TEXT;
ALTER TABLE push_subscriptions ADD COLUMN set_aside_why TEXT;
ALTER TABLE push_subscriptions ADD COLUMN last_probe_at TEXT;

ALTER TABLE jobs_notice_device ADD COLUMN strikes INTEGER NOT NULL DEFAULT 0 CHECK (strikes >= 0);
ALTER TABLE jobs_notice_device ADD COLUMN set_aside_at TEXT;
ALTER TABLE jobs_notice_device ADD COLUMN set_aside_why TEXT;
ALTER TABLE jobs_notice_device ADD COLUMN last_probe_at TEXT;
