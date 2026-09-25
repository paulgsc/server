//! Whose engagement is this?
//!
//! Auth has not landed. Everything downstream — the charge ledger, the
//! subscription table, the intervention log — is keyed by subject anyway,
//! because retrofitting an owner onto rows that never had one means guessing,
//! and because the alternative is a schema that quietly assumes one person and
//! has to be migrated the day that stops being true.
//!
//! So this is the seam, and it is deliberately one type and one extractor. When
//! auth arrives, [`SubjectId::from_request_parts`] starts reading a validated
//! token instead of returning the singleton, and nothing else in the codebase
//! changes.

use crate::FileHostError;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;

/// The single subject this deployment serves until auth exists.
///
/// A constant rather than an empty string or a zero: it is greppable, it is
/// obviously a placeholder in a database someone is looking at, and it cannot
/// be confused with a real identifier.
pub const SINGLETON_SUBJECT: &str = "subject-local";

/// Who a request is acting for.
///
/// Only this module can make one (#372). The extractor below is the single
/// place a request's subject is decided, and the day auth lands it is the
/// single place a token is checked — which only holds if nothing else can
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
/// let _ = file_host::subject::SubjectId::singleton();
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
	/// The one this deployment has until accounts exist.
	fn singleton() -> Self {
		Self(SINGLETON_SUBJECT.to_owned())
	}

	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}
}

// `axum-core` 0.4 still defines this trait through `async_trait`, so the impl
// has to be written the same way even though the body has nothing to await.
#[axum::async_trait]
impl<S: Send + Sync> FromRequestParts<S> for SubjectId {
	type Rejection = FileHostError;

	async fn from_request_parts(_parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
		// The whole of "auth" today. An extractor rather than a constant read
		// at each call site so that the day it becomes a token check, it is one
		// function body and not a search-and-replace.
		Ok(Self::singleton())
	}
}
