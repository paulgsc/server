-- Reverses 20261117000100_add_pairing_to_auth_session.up.sql. The sessions
-- stay; they forget which are phones and when those paired.
DROP INDEX idx_auth_session_pairing;
ALTER TABLE auth_session DROP COLUMN paired_on;
ALTER TABLE auth_session DROP COLUMN pairing_id;
ALTER TABLE auth_session DROP COLUMN kind;
