//! Passkey auth: accounts, passkeys and the sessions a sign-in hands out.
//!
//! See docs/identity.md, "Passkey auth", for the design and the privacy
//! invariants this module holds. In one line each:
//!
//! - A passkey is the only way in. No email, password, recovery code or other
//!   credential exists, so there is nothing about a person to store.
//! - An account is a random subject id, the WebAuthn user handle (16 random
//!   bytes) its passkeys carry, and those passkeys' public keys.
//! - Registration asks for `attestation: "none"`, and anything a client sends
//!   anyway is stripped before storage ([`passkey::for_storage`]), so no
//!   authenticator model is kept (invariant 7).
//! - A session is a 32-byte random token in an `HttpOnly; Secure;
//!   SameSite=Strict` cookie. Only its SHA-256 is stored ([`cookie`]).
//! - A state-changing, cookie-authenticated request must come from a trusted
//!   origin ([`csrf`]): `SameSite=Strict` alone admits same-site siblings.
//! - The pre-auth subject goes to a new account only with the operator's
//!   one-time `AUTH_LEGACY_CLAIM_TOKEN`, never to whoever registers first.
//! - [`crate::subject::SubjectId`]'s extractor is the one place a request's
//!   session becomes a subject. Everything downstream is unchanged by auth.
//! - An operator is a subject listed in `OPERATOR_SUBJECTS`, nothing more
//!   ([`operator::Operator`]). Signing in makes nobody an operator: anyone can
//!   make an account.
//!
//! [`AuthContext`] is its own state, bounded on `AuthContext: FromRef<S>` only,
//! per the convention docs/identity.md records: the auth routes and their tests
//! need a pool and this context, never the NATS connection `AppState` holds.

pub mod ceremony;
pub mod cookie;
pub mod csrf;
pub mod operator;
pub mod passkey;

use crate::{redacted::Redacted, Config, FileHostError};
use auth_repo::AuthRepository;
use axum::http::{HeaderMap, Method};
use ceremony::CeremonyStore;
use cookie::SessionToken;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};
use webauthn_rs::prelude::{Url, Webauthn, WebauthnBuilder};

/// What the relying party calls itself in a passkey prompt.
///
/// Also the fixed `user.name` a passkey is saved under: a name a person typed
/// would travel to their authenticator and, from there, to anything that syncs
/// it.
pub const PRODUCT_NAME: &str = "Some UI";

/// Everything the auth routes and the [`crate::subject::SubjectId`] extractor
/// need, and nothing else.
#[derive(Clone)]
pub struct AuthContext {
	inner: Arc<Inner>,
}

struct Inner {
	pool: SqlitePool,
	/// `None` when `AUTH_ENABLED=false`: the ceremony routes answer
	/// `503 feature_not_configured`, and with no way to sign in, every
	/// subject-scoped route answers `401`.
	relying_party: Option<Webauthn>,
	ceremonies: CeremonyStore,
	settings: Settings,
	signups: SignupCap,
	deletion: DeletionLock,
}

/// Orders account deletion against everything that writes subject-scoped
/// rows (`docs/identity.md` invariant 9).
///
/// The subject tables have no foreign key to `account` to refuse a write for
/// a deleted subject, so this lock is what refuses it. A deletion holds the
/// write guard for its one transaction. A writer holds a read guard across
/// its check that the subject still exists and its write:
/// - a request, from before its session is looked up until it finishes
///   (`subject::SubjectId`);
/// - the nudge waker, around each write plus a re-check of the subject's gate
///   row (`nudge::waker::unless_deleted`), never across a push delivery.
///
/// Every hold is short, and the lock is fair, so nothing waits behind more
/// than one deletion, and a deletion never waits for more than the writes in
/// flight.
#[derive(Clone, Default)]
pub struct DeletionLock(Arc<RwLock<()>>);

impl DeletionLock {
	/// A writer's hold. See the type's docs.
	pub(crate) async fn hold(&self) -> OwnedRwLockReadGuard<()> {
		Arc::clone(&self.0).read_owned().await
	}

	/// A deletion's hold: waits for every writer in flight, and holds off
	/// new ones until it is dropped.
	pub(crate) async fn exclude(&self) -> OwnedRwLockWriteGuard<()> {
		Arc::clone(&self.0).write_owned().await
	}
}

/// What `from_config` reads, gathered so tests can build one directly.
struct Settings {
	session_ttl_seconds: i64,
	new_accounts_per_day: u32,
	/// Origins a state-changing, cookie-authenticated request may come from:
	/// `ALLOWED_ORIGINS` and `WEBAUTHN_ORIGINS` together. See [`csrf`].
	trusted_origins: Vec<String>,
	/// `AUTH_LEGACY_CLAIM_TOKEN`: the one-time secret that lets a new account
	/// take the pre-auth subject. `None` means nobody can.
	legacy_claim: Option<Redacted<String>>,
	/// `OPERATOR_SUBJECTS`, trimmed, empties dropped. Empty means nobody is an
	/// operator. See [`operator`].
	operators: Vec<String>,
}

/// A ceiling on accounts created per UTC day, across the whole server.
///
/// Anyone who can reach the server may make an account, and the per-client
/// rate limiter on `/api/v1` already bounds how fast one client can. This is
/// the backstop for many clients (or a forged `X-Forwarded-For`, which buys a
/// new rate-limit bucket): a flood of empty accounts stops at this many a day.
/// Held in memory on purpose, so no column records when an account was made;
/// a restart resets the count, which only matters to someone already under
/// the cap.
struct SignupCap {
	per_day: u32,
	used: Mutex<(i64, u32)>,
}

impl SignupCap {
	const fn new(per_day: u32) -> Self {
		Self {
			per_day,
			used: Mutex::new((0, 0)),
		}
	}

	fn has_room(&self, day: i64) -> bool {
		let used = self.used.lock().unwrap_or_else(PoisonError::into_inner);
		used.0 != day || used.1 < self.per_day
	}

	/// Take one slot for `day`, or say there is none left.
	fn take(&self, day: i64) -> bool {
		let mut used = self.used.lock().unwrap_or_else(PoisonError::into_inner);
		if used.0 != day {
			*used = (day, 0);
		}
		if used.1 >= self.per_day {
			return false;
		}
		used.1 += 1;
		true
	}
}

/// Why the relying party could not be built from the configuration.
#[derive(Debug, thiserror::Error)]
pub enum RelyingPartyError {
	#[error(
		"passkey sign-in is on (AUTH_ENABLED defaults to true) but WEBAUTHN_RP_ID is unset: set WEBAUTHN_RP_ID and WEBAUTHN_ORIGINS, or AUTH_ENABLED=false to run with nobody able to sign in"
	)]
	NotConfigured,
	#[error("AUTH_ENABLED=false but WEBAUTHN_RP_ID is set: unset one of them")]
	DisabledButConfigured,
	#[error("WEBAUTHN_RP_ID is set but WEBAUTHN_ORIGINS is empty")]
	NoOrigins,
	#[error("WEBAUTHN_ORIGINS has an entry that is not a URL")]
	BadOrigin,
	#[error("the WebAuthn relying party could not be built: {0}")]
	Webauthn(#[from] webauthn_rs::prelude::WebauthnError),
}

impl AuthContext {
	/// Build from configuration.
	///
	/// Like the study nudge with `NUDGE_ENABLED` on, auth that is on but not
	/// configured is a startup error, not a warning. It used to boot and log
	/// one: the server looked up while every sign-in failed with "Passkey
	/// sign-in isn't set up on this server yet", which is discovering a
	/// missing variable from a user's screen. Running without sign-in is an
	/// explicit `AUTH_ENABLED=false`, and is logged.
	///
	/// # Errors
	/// When auth is on and `WEBAUTHN_RP_ID` is unset, when it is off and
	/// `WEBAUTHN_RP_ID` is set anyway, and when the origins are missing or
	/// malformed.
	pub fn from_config(pool: SqlitePool, config: &Config) -> Result<Self, RelyingPartyError> {
		let relying_party = configured_relying_party(config.auth_enabled, config.webauthn_rp_id.as_deref(), &config.webauthn_origins)?;
		if relying_party.is_none() {
			tracing::warn!("AUTH_ENABLED=false: passkey sign-in is off; nobody can sign in, and subject-scoped routes will answer 401");
		}
		let operators = operator::parse_subjects(&config.operator_subjects);
		if operators.is_empty() {
			tracing::warn!("OPERATOR_SUBJECTS is unset; the operator routes (lesson and LeetType round writes) are disabled and answer 403 to everyone");
		} else {
			tracing::info!(operators = operators.len(), "operator routes are enabled for the subjects in OPERATOR_SUBJECTS");
		}
		let trusted_origins = config
			.allowed_origins
			.iter()
			.chain(&config.webauthn_origins)
			.map(|origin| origin.trim().to_owned())
			.collect();
		Ok(Self::new(
			pool,
			relying_party,
			Settings {
				session_ttl_seconds: i64::from(config.auth_session_days) * 86_400,
				new_accounts_per_day: config.auth_new_accounts_per_day,
				trusted_origins,
				legacy_claim: config.auth_legacy_claim_token.clone().filter(|token| !token.is_empty()).map(Redacted::new),
				operators,
			},
		))
	}

	fn new(pool: SqlitePool, relying_party: Option<Webauthn>, settings: Settings) -> Self {
		Self {
			inner: Arc::new(Inner {
				pool,
				relying_party,
				ceremonies: CeremonyStore::default(),
				signups: SignupCap::new(settings.new_accounts_per_day),
				settings,
				deletion: DeletionLock::default(),
			}),
		}
	}

	#[cfg(test)]
	fn test_settings(new_accounts_per_day: u32, legacy_claim: Option<&str>) -> Settings {
		Settings {
			session_ttl_seconds: 30 * 86_400,
			new_accounts_per_day,
			trusted_origins: vec![String::from("https://app.test")],
			legacy_claim: legacy_claim.map(|token| Redacted::new(token.to_owned())),
			operators: Vec::new(),
		}
	}

	/// A context for tests, configured for `https://app.test`.
	#[cfg(test)]
	pub(crate) fn for_tests(pool: SqlitePool, new_accounts_per_day: u32) -> Self {
		Self::for_tests_with_claim(pool, new_accounts_per_day, None)
	}

	/// A context for tests whose `AUTH_LEGACY_CLAIM_TOKEN` is `legacy_claim`.
	#[cfg(test)]
	pub(crate) fn for_tests_with_claim(pool: SqlitePool, new_accounts_per_day: u32, legacy_claim: Option<&str>) -> Self {
		let relying_party = relying_party("app.test", &[String::from("https://app.test")]).unwrap();
		Self::new(pool, Some(relying_party), Self::test_settings(new_accounts_per_day, legacy_claim))
	}

	/// A context for tests whose `OPERATOR_SUBJECTS` is `operators`.
	#[cfg(test)]
	pub(crate) fn for_tests_with_operators(pool: SqlitePool, operators: &[&str]) -> Self {
		let relying_party = relying_party("app.test", &[String::from("https://app.test")]).unwrap();
		let mut settings = Self::test_settings(100, None);
		settings.operators = operators.iter().map(|&subject| subject.to_owned()).collect();
		Self::new(pool, Some(relying_party), settings)
	}

	/// A context for tests, with `WEBAUTHN_RP_ID` unset.
	#[cfg(test)]
	pub(crate) fn unconfigured_for_tests(pool: SqlitePool) -> Self {
		Self::new(pool, None, Self::test_settings(100, None))
	}

	pub(crate) fn repository(&self) -> AuthRepository {
		AuthRepository::new(self.inner.pool.clone())
	}

	pub(crate) fn pool(&self) -> &SqlitePool {
		&self.inner.pool
	}

	/// Whether a stored session may stand for anyone. Not with
	/// `AUTH_ENABLED=false`: a server that cannot sign anyone in must not keep
	/// letting in the sessions it issued while it could.
	pub(crate) fn sessions_enabled(&self) -> bool {
		self.inner.relying_party.is_some()
	}

	pub(crate) fn relying_party(&self) -> Result<&Webauthn, FileHostError> {
		self.inner.relying_party.as_ref().ok_or(FileHostError::FeatureNotConfigured("passkey auth"))
	}

	pub(crate) fn ceremonies(&self) -> &CeremonyStore {
		&self.inner.ceremonies
	}

	pub(crate) fn session_ttl_seconds(&self) -> i64 {
		self.inner.settings.session_ttl_seconds
	}

	/// The lock account deletion shares with every writer of subject-scoped
	/// rows; the nudge waker gets its copy from here.
	#[must_use]
	pub fn deletion_lock(&self) -> DeletionLock {
		self.inner.deletion.clone()
	}

	/// Held by a request acting for a subject, for as long as it runs. See
	/// [`DeletionLock`].
	pub(crate) async fn hold_against_deletion(&self) -> OwnedRwLockReadGuard<()> {
		self.inner.deletion.hold().await
	}

	/// Held by an account deletion. See [`DeletionLock::exclude`].
	pub(crate) async fn exclude_requests(&self) -> OwnedRwLockWriteGuard<()> {
		self.inner.deletion.exclude().await
	}

	/// Whether `GET /auth/session` may extend the session it reports.
	///
	/// Extending changes state, and a GET is exempt from [`Self::check_origin`].
	/// Without this check a sibling page could keep a session alive forever by
	/// embedding the URL as an image. So renewal is held to the same test a
	/// state-changing request passes.
	pub(crate) fn may_renew(&self, headers: &HeaderMap) -> bool {
		csrf::allowed(&Method::POST, headers, &self.inner.settings.trusted_origins)
	}

	/// Refuse a state-changing request a trusted origin did not make.
	///
	/// # Errors
	/// `403` for a cross-origin request from an untrusted origin; see [`csrf`].
	pub(crate) fn check_origin(&self, method: &Method, headers: &HeaderMap) -> Result<(), FileHostError> {
		if csrf::allowed(method, headers, &self.inner.settings.trusted_origins) {
			Ok(())
		} else {
			Err(FileHostError::Forbidden)
		}
	}

	/// Whether `offered` is this deployment's legacy claim token. Always false
	/// when none is configured.
	pub(crate) fn legacy_claim_matches(&self, offered: &str) -> bool {
		self.inner.settings.legacy_claim.as_ref().is_some_and(|token| {
			// Compared as digests, so the comparison's timing says nothing
			// about how much of the token a guess got right.
			Sha256::digest(token.expose().as_bytes()) == Sha256::digest(offered.as_bytes())
		})
	}

	/// Whether `subject` is listed in `OPERATOR_SUBJECTS`. Always false when
	/// none is configured.
	pub(crate) fn is_operator(&self, subject: &str) -> bool {
		self.inner.settings.operators.iter().any(|operator| operator == subject)
	}

	pub(crate) fn signups_have_room(&self, now: i64) -> bool {
		self.inner.signups.has_room(now.div_euclid(86_400))
	}

	pub(crate) fn take_signup(&self, now: i64) -> bool {
		self.inner.signups.take(now.div_euclid(86_400))
	}

	/// The subject of the live session this request's cookie names, if any.
	///
	/// Only [`crate::subject`] turns this into a `SubjectId`.
	///
	/// # Errors
	/// A storage failure. A missing, malformed, unknown or expired cookie is
	/// `Ok(None)`, not an error, and so is every cookie while sessions are off
	/// ([`Self::sessions_enabled`]).
	pub(crate) async fn session_subject(&self, headers: &HeaderMap) -> Result<Option<String>, FileHostError> {
		if !self.sessions_enabled() {
			return Ok(None);
		}
		let Some(token) = SessionToken::from_headers(headers) else {
			return Ok(None);
		};
		let session = self.repository().live_session(&token.hash(), now()).await?;
		Ok(session.map(|session| session.subject_id))
	}
}

/// What `from_config` builds from `AUTH_ENABLED` and `WEBAUTHN_RP_ID`, or why
/// the server must not start.
fn configured_relying_party(enabled: bool, rp_id: Option<&str>, origins: &[String]) -> Result<Option<Webauthn>, RelyingPartyError> {
	match (enabled, rp_id) {
		(true, Some(rp_id)) => relying_party(rp_id, origins).map(Some),
		(true, None) => Err(RelyingPartyError::NotConfigured),
		(false, Some(_)) => Err(RelyingPartyError::DisabledButConfigured),
		(false, None) => Ok(None),
	}
}

fn relying_party(rp_id: &str, origins: &[String]) -> Result<Webauthn, RelyingPartyError> {
	let mut origins = origins.iter().map(|origin| Url::parse(origin.trim()).map_err(|_| RelyingPartyError::BadOrigin));
	let first = origins.next().ok_or(RelyingPartyError::NoOrigins)??;
	let mut builder = WebauthnBuilder::new(rp_id, &first)?.rp_name(PRODUCT_NAME);
	for origin in origins {
		builder = builder.append_allowed_origin(&origin?);
	}
	Ok(builder.build()?)
}

/// Libraries whose own logging below `warn` is never written, whatever
/// `RUST_LOG` asks for.
///
/// - `webauthn_rs` instruments its ceremonies at debug and trace level,
///   recording its arguments: the browser's whole credential, its id and client
///   data included.
/// - `hyper_util::client` and `reqwest` log every outbound connection at
///   debug and trace level, naming its scheme, host and port. For a push
///   delivery that is the push endpoint's host, which `validate()` lets a
///   client choose (docs/identity.md, invariant 14).
const QUIET_BELOW_WARN: [&str; 3] = ["webauthn_rs", "hyper_util::client", "reqwest"];

/// Whether an event or span may reach the log, whatever `RUST_LOG` asks for.
///
/// Only the libraries in `QUIET_BELOW_WARN` are held back, and only their
/// warnings and errors pass. `main`'s subscriber applies this as a filter of
/// its own, so turning on debug logging to chase something else cannot put a
/// credential id or a push endpoint's host in a log line.
#[must_use]
pub fn loggable(metadata: &tracing::Metadata<'_>) -> bool {
	!QUIET_BELOW_WARN.iter().any(|quiet| metadata.target().starts_with(quiet)) || *metadata.level() <= tracing::Level::WARN
}

/// Unix seconds, now.
pub(crate) fn now() -> i64 {
	chrono::Utc::now().timestamp()
}

#[cfg(test)]
mod tests {
	use super::{configured_relying_party, relying_party, RelyingPartyError, SignupCap};

	#[test]
	fn the_signup_cap_refills_at_the_day_boundary_and_not_before() {
		let cap = SignupCap::new(2);
		assert!(cap.take(10) && cap.take(10));
		assert!(!cap.has_room(10));
		assert!(!cap.take(10), "a third account on the same day is refused");
		assert!(cap.has_room(11));
		assert!(cap.take(11), "the next day starts from zero");
	}

	#[test]
	fn configured_auth_needs_an_origin_that_parses() {
		assert!(matches!(relying_party("app.test", &[]), Err(RelyingPartyError::NoOrigins)));
		assert!(matches!(relying_party("app.test", &[String::from("not a url")]), Err(RelyingPartyError::BadOrigin)));
		assert!(relying_party("app.test", &[String::from("https://app.test"), String::from(" https://app.test:5173")]).is_ok());
	}

	/// The screenshot this guards against: auth on, `WEBAUTHN_RP_ID` missing
	/// from the `.env`, and a server that booted anyway with nobody able to
	/// sign in. Now that is a startup error, and only an explicit
	/// `AUTH_ENABLED=false` runs without a relying party.
	#[test]
	fn auth_on_without_a_relying_party_id_does_not_start() {
		let origins = [String::from("https://app.test")];
		assert!(matches!(configured_relying_party(true, None, &origins), Err(RelyingPartyError::NotConfigured)));
		assert!(matches!(configured_relying_party(true, Some("app.test"), &origins), Ok(Some(_))));
		assert!(matches!(configured_relying_party(false, None, &[]), Ok(None)));
		assert!(matches!(
			configured_relying_party(false, Some("app.test"), &origins),
			Err(RelyingPartyError::DisabledButConfigured)
		));
	}
}
