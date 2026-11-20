-- An account's pairing scope (paulgsc/some-ui#3197, C751): 16 random bytes,
-- set the first time a phone pairs with the account and the same for every
-- pairing after, answered on `/auth/pairing/finish` as `account`. A phone
-- keeps what it knows home holds under it, so pairing again with the same
-- account (after a reinstall, say) keeps that knowledge, while another home
-- or another account starts fresh. Random, never derived from the subject id
-- or the user handle; it goes with the account row (invariant 9).
ALTER TABLE account ADD COLUMN pairing_scope BLOB;
