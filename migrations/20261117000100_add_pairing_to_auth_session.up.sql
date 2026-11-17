-- A paired phone's session says it is one, and the day it paired (SIGNIN6,
-- paulgsc/some-ui#3092).
--
-- SUPERSEDES, for phones only, two things 20260927000200_create_passkey_auth
-- said and docs/identity.md held:
--
--   * "No column says a session is a phone's." `kind` now does. A phone's
--     pairing must outlive the per-account cap of 32 browser sessions, and the
--     cap can only spare a row that says what it is.
--   * "Deliberately absent: any creation or last-seen time." `paired_on` is a
--     creation day, for phones only, so the owner can tell their phones apart.
--
-- The owner, 2026-11-17: "yes, keep phone pairings past 32 and list them".
-- That is the decision invariant 10 (docs/identity.md) left to them. It is
-- taken as narrowly as it can be:
--
--   auth_session.kind       -- 'browser' or 'phone'. Every row already stored
--                              is 'browser'; a phone paired before this is
--                              found out the first time its bearer token is
--                              read, and tagged then.
--   auth_session.pairing_id -- 16 random bytes naming a phone to its owner, so
--                              one can be signed out. Never derived from the
--                              token or its hash. NULL for a browser.
--   auth_session.paired_on  -- the UTC day (YYYY-MM-DD) a phone paired. A day,
--                              not an instant. NULL for a browser, and for a
--                              phone paired before home kept the day.
--   auth_session.label      -- the name the owner typed for a phone, while
--                              pairing it or later from the list ("yes, let
--                              me name each phone too", the owner,
--                              2026-11-17). Only what they typed: trimmed, at
--                              most 40 characters, no control characters,
--                              nothing read off the device. NULL until named,
--                              and always for a browser.
--
-- Still absent: any last-seen or last-used time, on any row. When a phone was
-- last renewed is read off `expires_at`, which already holds it.

ALTER TABLE auth_session ADD COLUMN kind TEXT NOT NULL DEFAULT 'browser' CHECK (kind IN ('browser', 'phone'));
ALTER TABLE auth_session ADD COLUMN pairing_id BLOB;
ALTER TABLE auth_session ADD COLUMN paired_on TEXT;
ALTER TABLE auth_session ADD COLUMN label TEXT;

CREATE UNIQUE INDEX idx_auth_session_pairing ON auth_session (subject_id, pairing_id) WHERE pairing_id IS NOT NULL;
