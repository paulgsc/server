//! Whose request is this?
//!
//! Everything per-person — the charge ledger, the subscription table, the
//! intervention log, study sessions — is keyed by subject, and this is the one
//! place a request's subject is decided: [`SubjectId::from_request_parts`]
//! reads the session cookie and looks it up. A request with no live session is
//! refused with `401`, and a state-changing one from an untrusted origin with
//! `403` (`auth::csrf`); there is no fallback subject. Passkey auth changed this
//! module's extractor body and nothing downstream of it, as the seam was built
//! to allow (#252, #261, #372). See docs/identity.md.
//!
//! [`Delegated`] is the one other way in: an OAuth access token an AI service
//! holds for a subject, limited to the [`Scopes`] the subject approved. Only a
//! route that asks for `Delegated` accepts one; every route that takes a
//! `SubjectId` keeps reading the session cookie and nothing else, so no token
//! can sign out, add a passkey or delete an account.

use crate::auth::oauth::{hash_token, Scope, Scopes};
use crate::{auth::AuthContext, FileHostError};
use auth_repo::OAuthRepository;
use axum::extract::{FromRef, FromRequestParts};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use rand::RngCore;
use std::sync::Arc;
use tokio::sync::OwnedRwLockReadGuard;

/// The subject every request acted for before auth existed.
///
/// Rows written then carry it. A new account takes it over only with the
/// operator's one-time `AUTH_LEGACY_CLAIM_TOKEN` ([`legacy_subject`]), never
/// by being first: on a reachable server, first is whoever gets there. It is
/// otherwise an ordinary subject id, and no request is assigned it by default.
pub const SINGLETON_SUBJECT: &str = "subject-local";

/// Tables whose rows belong to one subject, and what they hold.
///
/// Each is keyed by a `subject_id` column. Deleting an account deletes the subject's rows
/// from every one of them (`handlers::auth::delete_account`), and the privacy
/// schema test fails if a migrated table has a `subject_id` and is missing
/// here, so a new per-person table cannot outlive its account by omission.
pub const SUBJECT_SCOPED_TABLES: &[(&str, &str)] = &[
	("account", "the account itself: a subject id and its WebAuthn user handle"),
	("activity_outcome", "per-block study outcomes"),
	("auth_session", "hashes of the account's sign-in sessions, and when they expire"),
	("engagement_charge", "engagement model state"),
	("engagement_gate", "the waker's due index"),
	("intervention_log", "what the waker decided and when"),
	(
		"learner_shelf",
		"items a learner generated and chose to keep (#387): key, content hash, when kept, and the body verbatim — nothing derived from play; see docs/identity.md",
	),
	(
		"oauth_access_token",
		"hashes of access tokens an AI service holds for the subject, their permissions and expiry",
	),
	(
		"oauth_grant",
		"which AI services the subject approved, for what, and hashed refresh tokens; see docs/identity.md",
	),
	("passkey", "the account's passkeys: public keys and counters, no attestation"),
	("presence_leases", "which session a subject is looking at, right now"),
	(
		"push_subscriptions",
		"a browser's push endpoint — stable per browser, and known to its push service; see docs/identity.md",
	),
	("sessions", "study sessions"),
];

/// Who a request is acting for.
///
/// Only this module can make one (#372). The extractor below is the single
/// place a request's subject is decided, which only holds if nothing else can
/// construct a `SubjectId` and hand it to a repository. Code outside this
/// module that tries does not compile. (The first example does compile, so a
/// `compile_fail` below can only be failing on the constructor, not on a
/// wrong path.)
///
/// ```
/// fn accepts(subject: &file_host::subject::SubjectId) -> &str {
///     subject.as_str()
/// }
/// ```
///
/// ```compile_fail
/// let _: file_host::subject::SubjectId = serde_json::from_str("\"subject-forged\"").unwrap();
/// ```
///
/// ```compile_fail
/// let _ = file_host::subject::SubjectId { id: "subject-forged".to_owned(), _hold: todo!() };
/// ```
///
/// It also carries the request's hold against the account being deleted
/// while the request runs (`AuthContext::hold_against_deletion`), so a write
/// a handler makes through it cannot outlive the account.
///
/// Background work that already holds a stored subject id (the waker reads
/// them back from `engagement_gate`) works with that `&str` directly; it is
/// not deciding who a request is for, so it has no reason to mint one.
#[derive(Debug, Clone)]
pub struct SubjectId {
	id: String,
	_hold: Hold,
}

/// A request's hold against its account being deleted while it runs
/// (`AuthContext::hold_against_deletion`). Released when the last clone of
/// the `SubjectId` carrying it is dropped.
#[derive(Debug, Clone)]
struct Hold {
	_guard: Arc<OwnedRwLockReadGuard<()>>,
}

impl SubjectId {
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.id
	}

	/// The subject id, with this request's hold against deletion released.
	/// Only an account deletion needs this, before it waits for every other
	/// hold to be released.
	pub(crate) fn release(self) -> String {
		self.id
	}
}

impl PartialEq for SubjectId {
	fn eq(&self, other: &Self) -> bool {
		self.id == other.id
	}
}

impl Eq for SubjectId {}

/// The subject id for a new account: `subject-` and 32 random hex digits.
/// Nothing about the person, the request, or the time goes into it.
#[must_use]
pub fn new_account_subject() -> String {
	let mut bytes = [0_u8; 16];
	rand::rng().fill_bytes(&mut bytes);
	String::from("subject-") + &hex::encode(bytes)
}

/// The subject a new account takes when it presents the operator's
/// `AUTH_LEGACY_CLAIM_TOKEN`: the one the pre-auth rows were written under.
#[must_use]
pub const fn legacy_subject() -> &'static str {
	SINGLETON_SUBJECT
}

// `axum-core` 0.4 still defines this trait through `async_trait`, so the impl
// has to be written the same way.
#[axum::async_trait]
impl<S> FromRequestParts<S> for SubjectId
where
	AuthContext: FromRef<S>,
	S: Send + Sync,
{
	type Rejection = FileHostError;

	async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
		let auth = AuthContext::from_ref(state);
		// A same-site sibling page can make the browser attach the cookie to
		// a form post; see `auth::csrf`.
		auth.check_origin(&parts.method, &parts.headers)?;
		// Taken before the session is looked up: once a deletion has
		// committed, a request that gets a hold afterwards finds no session.
		let hold = Hold {
			_guard: Arc::new(auth.hold_against_deletion().await),
		};
		let subject = auth.session_subject(&parts.headers).await?.ok_or(FileHostError::Unauthorized)?;
		Ok(Self { id: subject, _hold: hold })
	}
}

/// A request an AI service makes for a subject, with an OAuth access token
/// the subject approved (docs/identity.md, "AI services acting for a
/// subject").
///
/// Refused with `401` unless the `Authorization: Bearer` token is live, was
/// issued for this server's MCP endpoint (`OAUTH_RESOURCE`) and OAuth is on.
/// No origin check: a bearer token is not ambient, so no other page can make
/// a browser attach it. It carries the same hold against account deletion as
/// a [`SubjectId`], taken before the token is looked up.
#[derive(Debug, Clone)]
pub struct Delegated {
	subject: SubjectId,
	scopes: Scopes,
}

impl Delegated {
	#[must_use]
	pub const fn subject(&self) -> &SubjectId {
		&self.subject
	}

	/// Whether the subject approved `scope` for this service.
	#[must_use]
	pub fn allows(&self, scope: Scope) -> bool {
		self.scopes.contains(scope)
	}
}

#[axum::async_trait]
impl<S> FromRequestParts<S> for Delegated
where
	AuthContext: FromRef<S>,
	S: Send + Sync,
{
	type Rejection = FileHostError;

	async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
		let auth = AuthContext::from_ref(state);
		let resource = auth.oauth().map_err(|_| FileHostError::Unauthorized)?.settings.resource.clone();
		let token = parts
			.headers
			.get(AUTHORIZATION)
			.and_then(|value| value.to_str().ok())
			// The scheme is case-insensitive (RFC 7235 §2.1).
			.and_then(|value| value.split_once(' '))
			.filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
			.map(|(_, token)| token.trim())
			.filter(|token| !token.is_empty())
			.ok_or(FileHostError::Unauthorized)?;
		let hold = Hold {
			_guard: Arc::new(auth.hold_against_deletion().await),
		};
		let row = OAuthRepository::new(auth.pool().clone())
			.access_token(&hash_token(token), crate::auth::now())
			.await?
			.filter(|row| row.resource == resource)
			.ok_or(FileHostError::Unauthorized)?;
		let scopes = Scopes::parse(&row.scope).ok_or(FileHostError::Unauthorized)?;
		Ok(Self {
			subject: SubjectId { id: row.subject_id, _hold: hold },
			scopes,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::{legacy_subject, new_account_subject};

	#[test]
	fn a_new_subject_is_random_and_names_nothing() {
		let a = new_account_subject();
		let b = new_account_subject();
		assert_eq!(legacy_subject(), "subject-local");
		assert_ne!(a, b);
		assert_eq!(a.len(), "subject-".len() + 32);
		assert!(a.strip_prefix("subject-").unwrap().bytes().all(|byte| byte.is_ascii_hexdigit()));
	}
}
