-- Passkey auth: an account is a random subject, the passkeys that open it, and
-- the sessions a sign-in hands out. See docs/identity.md, "Passkey auth".
--
-- Nothing here names a person. Each column is what a sign-in needs and no more:
--
--   account.subject_id      -- the `SubjectId` every per-person table is keyed
--                              by. Random, minted by `subject.rs`; the first
--                              account on a deployment is handed the
--                              pre-auth placeholder so it inherits that data.
--   account.user_handle     -- the WebAuthn `user.id`: 16 random bytes, and
--                              the only thing an authenticator hands back on a
--                              username-less sign-in. Kept separate from
--                              subject_id so the placeholder can be claimed
--                              without a user handle ever carrying it.
--   passkey.credential_id   -- the authenticator's own ID for the passkey.
--   passkey.passkey         -- the public key and signature counter, as the
--                              WebAuthn library serialises them. Registered
--                              with `attestation: "none"`, so it carries no
--                              authenticator model.
--   auth_session.token_hash -- SHA-256 of the session cookie. The cookie
--                              itself is never stored, so a copy of this
--                              database opens no session.
--   auth_session.expires_at -- unix seconds. Sessions end on their own.
--
-- Deliberately absent: any creation or last-seen time on an account or a
-- passkey. Nothing needs one, and a timeline of when someone uses the app is
-- exactly what this schema should not be able to answer. No REFERENCES,
-- matching every other table in this schema; account deletion removes a
-- subject's rows from every subject-scoped table by name instead.

CREATE TABLE account (
	subject_id TEXT PRIMARY KEY NOT NULL,
	user_handle BLOB NOT NULL UNIQUE
);

CREATE TABLE passkey (
	credential_id BLOB PRIMARY KEY NOT NULL,
	subject_id TEXT NOT NULL,
	passkey TEXT NOT NULL
);

CREATE INDEX idx_passkey_subject ON passkey (subject_id);

CREATE TABLE auth_session (
	token_hash BLOB PRIMARY KEY NOT NULL,
	subject_id TEXT NOT NULL,
	expires_at INTEGER NOT NULL
);

CREATE INDEX idx_auth_session_subject ON auth_session (subject_id, expires_at);
CREATE INDEX idx_auth_session_expires_at ON auth_session (expires_at);
