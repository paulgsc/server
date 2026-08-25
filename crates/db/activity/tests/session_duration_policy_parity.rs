//! The parity test between `activity_repo::provisioning`'s transcribed
//! client constants and `DEFAULT_SESSION_DURATION_POLICY`
//! (`apps/www/src/lib/session-duration-policy/index.ts`, `paulgsc/some-ui`).
//! #281 (RCM4).
//!
//! ## The user story
//!
//! "As the person who tightens or loosens the client's session duration
//! policy, I want to find out at test time that the server's copy of it is
//! now wrong, not from a provisioned session the client's own composer
//! rejects on open."
//!
//! ## Mechanism
//!
//! Same shape `tests/catalog_parity.rs` already established for
//! `min_duration_ms`: a checked-in fixture
//! (`testdata/session_duration_policy.snapshot.json`), exported from the
//! client by `paulgsc/some-ui`'s `scripts/dump-session-duration-policy.ts`
//! (`pnpm dump:session-duration-policy > session-duration-policy.snapshot.json`,
//! then copied here by hand), compared against the constants this crate
//! actually uses. No live database needed -- these are two plain integers,
//! not a table.
//!
//! If this test fails after touching `DEFAULT_SESSION_DURATION_POLICY`,
//! regenerate the fixture (see `dump-session-duration-policy.ts`'s own doc
//! comment) and update `CLIENT_MIN_ACTIVITY_DURATION_MS`/
//! `CLIENT_MAX_TOTAL_DURATION_MS` in `src/provisioning.rs` to match.

use activity_repo::{CLIENT_MAX_TOTAL_DURATION_MS, CLIENT_MIN_ACTIVITY_DURATION_MS};
use serde::Deserialize;

/// `SessionDurationPolicy`
/// (`apps/www/src/lib/session-duration-policy/index.ts`, `paulgsc/some-ui`).
/// `deny_unknown_fields` for the same reason `catalog_parity.rs`'s
/// `FixtureActivity` uses it: a third field added to the client's policy
/// and forgotten here should fail loudly, not parse silently into nothing.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixturePolicy {
	min_activity_duration_ms: i64,
	max_total_duration_ms: i64,
}

#[test]
fn the_transcribed_client_constants_match_the_clients_own_snapshot() {
	// `.unwrap()`, not `.expect()`: `clippy.toml`'s `allow-unwrap-in-tests`
	// covers the former only.
	let fixture: FixturePolicy = serde_json::from_str(include_str!("../testdata/session_duration_policy.snapshot.json")).unwrap();

	assert_eq!(
		CLIENT_MIN_ACTIVITY_DURATION_MS, fixture.min_activity_duration_ms,
		"CLIENT_MIN_ACTIVITY_DURATION_MS ({CLIENT_MIN_ACTIVITY_DURATION_MS}) diverged from the client's DEFAULT_SESSION_DURATION_POLICY.minActivityDurationMs ({}) -- update provisioning.rs's transcription",
		fixture.min_activity_duration_ms
	);
	assert_eq!(
		CLIENT_MAX_TOTAL_DURATION_MS, fixture.max_total_duration_ms,
		"CLIENT_MAX_TOTAL_DURATION_MS ({CLIENT_MAX_TOTAL_DURATION_MS}) diverged from the client's DEFAULT_SESSION_DURATION_POLICY.maxTotalDurationMs ({}) -- update provisioning.rs's transcription",
		fixture.max_total_duration_ms
	);
}
