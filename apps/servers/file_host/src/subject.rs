//! Whose request is this?
//!
//! Everything per-person — the charge ledger, the subscription table, the
//! intervention log, study sessions — is keyed by subject, and this is the one
//! place a request's subject is decided: [`SubjectId::from_request_parts`]
//! reads the session cookie and looks it up. A request with no live session is
//! refused with `401`; there is no fallback subject. Passkey auth changed this
//! module's extractor body and nothing downstream of it, as the seam was built
//! to allow (#252, #261, #372). See docs/identity.md.

use crate::{auth::AuthContext, FileHostError};
use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use rand::RngCore;

/// The subject every request acted for before auth existed.
///
/// Rows written then carry it, so the first account a deployment creates is
/// given this id and inherits them ([`new_account_subjects`]). After that it
/// is an ordinary subject id, and no request is assigned it by default.
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
/// let _ = file_host::subject::SubjectId("subject-forged".to_owned());
/// ```
///
/// Background work that already holds a stored subject id (the waker reads
/// them back from `engagement_gate`) works with that `&str` directly; it is
/// not deciding who a request is for, so it has no reason to mint one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectId(String);

impl SubjectId {
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}
}

/// The two subject ids a new account can get.
///
/// `if_first` when it is the first account on this deployment, `otherwise`
/// when it is not. The repository makes that choice atomically; this decides
/// only what the two candidates are.
///
/// `otherwise` is `subject-` and 32 random hex digits. Nothing about the
/// person, the request, or the time goes into it.
#[must_use]
pub fn new_account_subjects() -> (&'static str, String) {
	let mut bytes = [0_u8; 16];
	rand::rng().fill_bytes(&mut bytes);
	(SINGLETON_SUBJECT, String::from("subject-") + &hex::encode(bytes))
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
		AuthContext::from_ref(state)
			.session_subject(&parts.headers)
			.await?
			.map(Self)
			.ok_or(FileHostError::Unauthorized)
	}
}

#[cfg(test)]
mod tests {
	use super::new_account_subjects;

	#[test]
	fn a_new_subject_is_random_and_names_nothing() {
		let (first, a) = new_account_subjects();
		let (_, b) = new_account_subjects();
		assert_eq!(first, "subject-local");
		assert_ne!(a, b);
		assert_eq!(a.len(), "subject-".len() + 32);
		assert!(a.strip_prefix("subject-").unwrap().bytes().all(|byte| byte.is_ascii_hexdigit()));
	}
}
