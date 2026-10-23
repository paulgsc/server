//! Whose request is this?
//!
//! Everything per-person — the charge ledger, the subscription table, the
//! intervention log, study sessions — is keyed by subject, and this is the one
//! place a request's subject is decided: [`SubjectId::from_request_parts`]
//! reads the session (a browser's cookie, or a paired phone's bearer token)
//! and looks it up. [`BrowserSubject`] narrows it to a browser's passkey
//! sign-in, for what a phone's token must not reach. A request with no live
//! session is refused with `401`, and a state-changing one from an untrusted
//! origin with `403` (`auth::csrf`); there is no fallback subject. Passkey auth changed this
//! module's extractor body and nothing downstream of it, as the seam was built
//! to allow (#252, #261, #372). See docs/identity.md.

use crate::{
	auth::{
		cookie::{pairing_of, Carrier, SessionToken},
		AuthContext,
	},
	FileHostError,
};
use axum::extract::{FromRef, FromRequestParts};
use axum::http::{request::Parts, HeaderMap};
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
	(
		"aph_entry",
		"the owner's aph log (paulgsc/some-ui `packages/ui/aph`), every revision of every entry: the figures, labels and note as written, and when each revision was written; see docs/identity.md",
	),
	("aph_settings", "the checkpoints, goals and labels the aph log is read against, every revision"),
	("auth_session", "hashes of the account's sign-in sessions, and when they expire"),
	("engagement_charge", "engagement model state"),
	("engagement_gate", "the waker's due index"),
	("intervention_log", "what the waker decided and when"),
	(
		"jobs_event",
		"the owner's Jobs log (paulgsc/some-ui `apps/www/src/lib/jobs`), every event: picks taken, passed or archived and why, why the owner fits one taken, how applying went, outcomes, postings found by hand, as written, and when each was recorded and reached home; see docs/identity.md",
	),
	(
		"learner_shelf",
		"items a learner generated and chose to keep (#387): key, content hash, when kept, and the body verbatim — nothing derived from play; see docs/identity.md",
	),
	(
		"leetype_session_event",
		"the owner's LeetType session log (paulgsc/some-ui `packages/ui/leetype/src/lib/leetype/session-log`), every event of every session sent home: the session file played, each pick and how long it took, readings placed, opened, saved or read, rounds parked, returned and confirmed, how each spoken answer was given and whether it was strong (never what was said, typed or ticked), how the session ended, and when each was recorded and reached home; see docs/identity.md",
	),
	(
		"mailbox_handed",
		"each answer the Jobs mailbox gave when home handed it one of the owner's files, and when; see docs/identity.md invariant 19",
	),
	(
		"mailbox_item",
		"the files home read from the Jobs mailbox for the owner (paulgsc/some-ui `apps/mailbox`), each the routine's daily job digest: its bytes as the mailbox held them, its hash, size and sequence number, and when home read it; see docs/identity.md invariant 19",
	),
	(
		"mailbox_outgoing",
		"the owner's voice-free Jobs decisions and scouts filed with home for the mailbox: their bytes and hash, and when each was filed; see docs/identity.md invariant 19",
	),
	("passkey", "the account's passkeys: public keys and counters, no attestation"),
	("presence_leases", "which session a subject is looking at, right now"),
	(
		"push_subscriptions",
		"a browser's push endpoint — stable per browser, and known to its push service; see docs/identity.md",
	),
	("sessions", "study sessions"),
	(
		"sharing_choice",
		"what each paired phone's person chose to keep sending home, and when each choice began and ended: a time per change, none per item sent; see docs/identity.md",
	),
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

/// A subject whose session is a browser's passkey sign-in (the cookie), not a
/// paired phone's bearer token, which answers `403` here.
///
/// A phone is paired to act on the account's study data. Adding a passkey,
/// pairing another phone, deleting the account and the operator routes would
/// outlive or widen the session that asked, and a phone's token sits in app
/// storage, easier to lift than an `HttpOnly` cookie. So none of them take
/// one (docs/identity.md, "Pairing a phone").
///
/// Sound because a session's kind is how it arrived: a phone's token is
/// stored under a digest no cookie matches (`auth::cookie::SessionToken::hash`).
#[derive(Debug)]
pub struct BrowserSubject(pub SubjectId);

#[axum::async_trait]
impl<S> FromRequestParts<S> for BrowserSubject
where
	AuthContext: FromRef<S>,
	S: Send + Sync,
{
	type Rejection = FileHostError;

	async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
		let subject = SubjectId::from_request_parts(parts, state).await?;
		match SessionToken::from_headers(&parts.headers) {
			Some((_, Carrier::Bearer)) => Err(FileHostError::Forbidden),
			_ => Ok(Self(subject)),
		}
	}
}

/// A paired phone's request: its subject, and its pairing, which a browser's
/// cookie session has none of, so it answers `403` here.
///
/// The pairing is named from its session (`auth::cookie::pairing_of`), never
/// from anything the phone says about itself (invariant 4), and it is no
/// identity: it only tells one pairing's choices from another's under the
/// same subject, as the phone's own consent log does.
#[derive(Debug)]
pub struct Paired {
	pub subject: SubjectId,
	pairing: String,
}

impl Paired {
	#[must_use]
	pub fn pairing(&self) -> &str {
		&self.pairing
	}
}

/// The pairing a request's bearer session names, or `None` for a browser's.
/// It reads the session `SubjectId` resolved, as both read the bearer first
/// (`SessionToken::from_headers`).
pub(crate) fn pairing_in(headers: &HeaderMap) -> Option<String> {
	match SessionToken::from_headers(headers) {
		Some((token, Carrier::Bearer)) => Some(pairing_of(&token.hash(Carrier::Bearer))),
		_ => None,
	}
}

#[axum::async_trait]
impl<S> FromRequestParts<S> for Paired
where
	AuthContext: FromRef<S>,
	S: Send + Sync,
{
	type Rejection = FileHostError;

	async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
		let subject = SubjectId::from_request_parts(parts, state).await?;
		let pairing = pairing_in(&parts.headers).ok_or(FileHostError::Forbidden)?;
		Ok(Self { subject, pairing })
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
