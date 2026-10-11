-- AI services acting for a subject (docs/identity.md, "AI services acting for
-- a subject"): OAuth 2.1 with `file_host` as its own authorization server.
--
-- No column records when anything was made or last used (invariant 10). An
-- expiry is the one instant stored, as on `auth_session`.

-- A service that registered itself (RFC 7591). About the service, never about
-- a person: corpus-wide, no subject column.
CREATE TABLE oauth_client (
	client_id     TEXT PRIMARY KEY NOT NULL,  -- random, minted here
	client_name   TEXT NOT NULL,              -- what the service called itself
	redirect_uris TEXT NOT NULL               -- JSON array, matched exactly
);

-- What a subject approved for a client. The refresh token is replaced on every
-- use; the one it replaced is kept so that presenting it again (only a copy
-- could) ends the grant.
CREATE TABLE oauth_grant (
	grant_id              TEXT PRIMARY KEY NOT NULL,  -- random
	subject_id            TEXT NOT NULL,
	client_id             TEXT NOT NULL,
	scope                 TEXT NOT NULL,              -- space-separated
	resource              TEXT NOT NULL,              -- the audience it was issued for
	refresh_hash          BLOB NOT NULL UNIQUE,       -- SHA-256 of the live refresh token
	previous_refresh_hash BLOB UNIQUE,                -- SHA-256 of the one it replaced
	expires_at            INTEGER NOT NULL            -- unix seconds; the refresh token's
);

CREATE INDEX idx_oauth_grant_subject ON oauth_grant (subject_id);
CREATE INDEX idx_oauth_grant_client ON oauth_grant (client_id);

CREATE TABLE oauth_access_token (
	token_hash BLOB PRIMARY KEY NOT NULL,  -- SHA-256 of the token
	grant_id   TEXT NOT NULL,
	subject_id TEXT NOT NULL,
	scope      TEXT NOT NULL,
	expires_at INTEGER NOT NULL            -- unix seconds
);

CREATE INDEX idx_oauth_access_token_grant ON oauth_access_token (grant_id);
CREATE INDEX idx_oauth_access_token_subject ON oauth_access_token (subject_id);

-- Invariant 12: a subject-scoped table newer than account deletion (#395)
-- leaves with the account even on a binary rolled back past this migration.
CREATE TRIGGER oauth_grant_leaves_with_account
AFTER DELETE ON account
BEGIN
	DELETE FROM oauth_grant WHERE subject_id = OLD.subject_id;
END;

CREATE TRIGGER oauth_access_token_leaves_with_account
AFTER DELETE ON account
BEGIN
	DELETE FROM oauth_access_token WHERE subject_id = OLD.subject_id;
END;
