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
//! - [`crate::subject::SubjectId`]'s extractor is the one place a request's
//!   session becomes a subject. Everything downstream is unchanged by auth.
//!
//! [`AuthContext`] is its own state, bounded on `AuthContext: FromRef<S>` only,
//! per the convention docs/identity.md records: the auth routes and their tests
//! need a pool and this context, never the NATS connection `AppState` holds.

pub mod ceremony;
pub mod cookie;
pub mod passkey;

use crate::{Config, FileHostError};
use auth_repo::AuthRepository;
use axum::http::HeaderMap;
use ceremony::CeremonyStore;
use cookie::SessionToken;
use sqlx::SqlitePool;
use std::sync::{Arc, Mutex, PoisonError};
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
	/// `None` when `WEBAUTHN_RP_ID` is unset: the ceremony routes answer
	/// `503 feature_not_configured`, and with no way to sign in, every
	/// subject-scoped route answers `401`.
	relying_party: Option<Webauthn>,
	ceremonies: CeremonyStore,
	session_ttl_seconds: i64,
	signups: SignupCap,
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
	/// Like the study nudge, auth that is simply not configured is not an
	/// error: the server boots and says so. Auth that is configured wrongly
	/// is, because the alternative is discovering it at someone's first
	/// sign-in.
	///
	/// # Errors
	/// When `WEBAUTHN_RP_ID` is set and the origins are missing or
	/// malformed.
	pub fn from_config(pool: SqlitePool, config: &Config) -> Result<Self, RelyingPartyError> {
		let relying_party = if let Some(rp_id) = config.webauthn_rp_id.as_deref() {
			Some(relying_party(rp_id, &config.webauthn_origins)?)
		} else {
			tracing::warn!("passkey auth is not configured (WEBAUTHN_RP_ID is unset); nobody can sign in, and subject-scoped routes will answer 401");
			None
		};
		Ok(Self::new(
			pool,
			relying_party,
			i64::from(config.auth_session_days) * 86_400,
			config.auth_new_accounts_per_day,
		))
	}

	fn new(pool: SqlitePool, relying_party: Option<Webauthn>, session_ttl_seconds: i64, new_accounts_per_day: u32) -> Self {
		Self {
			inner: Arc::new(Inner {
				pool,
				relying_party,
				ceremonies: CeremonyStore::default(),
				session_ttl_seconds,
				signups: SignupCap::new(new_accounts_per_day),
			}),
		}
	}

	/// A context for tests, configured for `https://app.test`.
	#[cfg(test)]
	pub(crate) fn for_tests(pool: SqlitePool, new_accounts_per_day: u32) -> Self {
		let relying_party = relying_party("app.test", &[String::from("https://app.test")]).unwrap();
		Self::new(pool, Some(relying_party), 30 * 86_400, new_accounts_per_day)
	}

	/// A context for tests, with `WEBAUTHN_RP_ID` unset.
	#[cfg(test)]
	pub(crate) fn unconfigured_for_tests(pool: SqlitePool) -> Self {
		Self::new(pool, None, 30 * 86_400, 100)
	}

	pub(crate) fn repository(&self) -> AuthRepository {
		AuthRepository::new(self.inner.pool.clone())
	}

	pub(crate) fn pool(&self) -> &SqlitePool {
		&self.inner.pool
	}

	pub(crate) fn relying_party(&self) -> Result<&Webauthn, FileHostError> {
		self.inner.relying_party.as_ref().ok_or(FileHostError::FeatureNotConfigured("passkey auth"))
	}

	pub(crate) fn ceremonies(&self) -> &CeremonyStore {
		&self.inner.ceremonies
	}

	pub(crate) fn session_ttl_seconds(&self) -> i64 {
		self.inner.session_ttl_seconds
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
	/// `Ok(None)`, not an error.
	pub(crate) async fn session_subject(&self, headers: &HeaderMap) -> Result<Option<String>, FileHostError> {
		let Some(token) = SessionToken::from_headers(headers) else {
			return Ok(None);
		};
		let session = self.repository().live_session(&token.hash(), now()).await?;
		Ok(session.map(|session| session.subject_id))
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

/// Whether an event or span may reach the log, whatever `RUST_LOG` asks for.
///
/// The WebAuthn library instruments its ceremonies at debug and trace level,
/// recording its arguments: the browser's whole credential, its id and client
/// data included. Only that library's warnings and errors pass. `main`'s
/// subscriber applies this as a filter of its own, so turning on debug logging
/// to chase something else cannot put a credential id in a log line.
#[must_use]
pub fn loggable(metadata: &tracing::Metadata<'_>) -> bool {
	!metadata.target().starts_with("webauthn_rs") || *metadata.level() <= tracing::Level::WARN
}

/// Unix seconds, now.
pub(crate) fn now() -> i64 {
	chrono::Utc::now().timestamp()
}

#[cfg(test)]
mod tests {
	use super::{relying_party, RelyingPartyError, SignupCap};

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
}
