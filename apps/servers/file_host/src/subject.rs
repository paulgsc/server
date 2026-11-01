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
		cookie::{Carrier, SessionToken},
		AuthContext,
	},
	FileHostError,
};
use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use rand::RngCore;
use std::sync::Arc;
use tokio::sync::OwnedRwLockReadGuard;

/// Tables whose rows belong to one subject, and what they hold.
///
/// Each is keyed by a `subject_id` column. Deleting an account deletes the subject's rows
/// from every one of them (`handlers::auth::delete_account`), and the privacy
/// schema test fails if a migrated table has a `subject_id` and is missing
/// here, so a new per-person table cannot outlive its account by omission.
/// A table here is also named, under the kind the person is told it is, in
/// docs/identity.md's "What deleting an account erases" (invariant 22).
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
	(
		"game_bite",
		"what each of the owner's game bites (a voice note said while watching a game) was said about and what the screen showed: the scenario and how it came to be said, the offer, the live strip and context line, whether they were watching, when they started saying it, the phone's clock offset, the mode and the scenario set's version; see docs/identity.md invariant 23",
	),
	("intervention_log", "what the waker decided and when"),
	(
		"jobs_event",
		"the owner's Jobs log (paulgsc/some-ui `apps/www/src/lib/jobs`), every event: picks taken, passed or archived and why, why the owner fits one taken, how applying went, outcomes, postings found by hand, as written, and when each was recorded and reached home; see docs/identity.md",
	),
	(
		"jobs_notice_device",
		"the owner's phones that asked to be told when a morning run's record arrives: each one's FCM registration token, which names that install of the app to Google, and when it asked; see docs/identity.md invariant 24",
	),
	(
		"jobs_notice_sent",
		"which of the owner's morning runs home told the phones of: the run record's name, what was said (picks, couldn't finish, or nothing for a re-run), to how many phones, and when; see docs/identity.md invariant 24",
	),
	(
		"korean_entry",
		"the owner's Korean record (paulgsc/some-ui#2100), every revision of every entry: the word sets they study on, each word's pace, each sentence's tally, the lens record, the practice record, the set left unfinished and their answers at a sitting's end, as the phone sent them, and when each revision was written; see docs/identity.md invariant 25",
	),
	(
		"learner_shelf",
		"items a learner generated and chose to keep (#387): key, content hash, when kept, and the body verbatim — nothing derived from play; see docs/identity.md",
	),
	(
		"leetype_session_event",
		"the owner's LeetType session log (paulgsc/some-ui `packages/ui/leetype/src/lib/leetype/session-log`), every event of every session sent home: the session file played, each pick and how long it took, readings placed, opened, saved or read, rounds parked, returned and confirmed, how each spoken answer was given and whether it was strong (never what was said, typed or ticked), which round a take's rubric was shown for (by name only), how the session ended, and when each was recorded and reached home; see docs/identity.md",
	),
	(
		"mailbox_archive",
		"the owner's files home archived from the Jobs mailbox, from any folder: their bytes as the mailbox held them, hash, size, sequence number, when each was first put and when home stored it; see docs/identity.md invariants 19 and 21",
	),
	(
		"mailbox_archive_answer",
		"each answer the Jobs mailbox gave when home asked it to drop one of the owner's written reflections or drama prompts home had archived, and when; see docs/identity.md invariant 21",
	),
	(
		"mailbox_handed",
		"each answer the Jobs mailbox gave when home handed it one of the owner's files, and when; see docs/identity.md invariant 19",
	),
	(
		"mailbox_item",
		"the files home read from the Jobs mailbox for the owner (paulgsc/some-ui `apps/mailbox`), each a routine's file (the Jobs digest, the reflections routine's word sets, dramas and run records, or a LeetType session file): its bytes as the mailbox held them, its hash, size and sequence number, and when home read it; see docs/identity.md invariant 19",
	),
	(
		"mailbox_leetype_log",
		"each decision home made on the owner's finished LeetType sessions for the mailbox: the session, the highest of its events the decision covered, and either the number its filed file's name carries or why it could not be filed (unreadable, no_file, too_big), and when; see docs/identity.md invariants 18 and 19",
	),
	(
		"mailbox_outgoing",
		"the owner's Jobs decisions and scouts (voice notes named by id only), which of the reflections routine's lessons they opened (names only), what home heard in their voice notes (text only), how each of their finished LeetType sessions went (every event home holds of it, never what was said, typed or ticked), their written reflections (the typed line, its session and when), and the drama prompts they asked for (the prompt's stamp and markdown), filed with home for the mailbox: their bytes and hash, and when each was filed; see docs/identity.md invariant 19",
	),
	(
		"mailbox_transcript",
		"which of what home heard in the owner's voice notes it filed for the mailbox: the note, the model that heard it, the number its file's name carries, and when; see docs/identity.md invariant 19",
	),
	("passkey", "the account's passkeys: public keys and counters, no attestation"),
	("presence_leases", "which session a subject is looking at, right now"),
	(
		"push_subscriptions",
		"a browser's push endpoint — stable per browser, and known to its push service; see docs/identity.md",
	),
	(
		"rust_round",
		"each Rust Rounds round the owner's phone put (paulgsc/some-ui#2234), as the tutor wrote it: its cards and programs, and a refine card's voice-note id and the tutor's nudge, which can paraphrase what the owner said; and when it reached home. See docs/identity.md invariant 28",
	),
	("sessions", "study sessions"),
	(
		"studio_friction",
		"how each of the owner's Studio sittings went (paulgsc/some-ui ST9): smooth, some snags or rough, the cause chosen or said (at most 200 characters, possibly typed), the sitting and its kind, the shot it was on and a voice note's id, and when each was recorded and reached home; see docs/identity.md invariant 26",
	),
	(
		"studio_sitting_event",
		"the owner's Studio time log (paulgsc/some-ui ST9), every event of every sitting sent home: the sitting and its kind, each screen entered and left by name, the page hidden and shown, the sitting opened and closed, and when each was recorded and reached home; see docs/identity.md invariant 26",
	),
	(
		"study_note",
		"the text the owner's phone keeps beside a soundbite or a LeetType spoken answer (paulgsc/some-ui#2180), as the phone sent it: a soundbite's record (when it was recorded, how long it ran, how it came to be recorded, the sessions the phone knew of and its time zone), and an answer's note (the session file and screen, the prompt, the rubric, the points ticked, the answer typed and the take it notes), and when each reached home; see docs/identity.md invariant 30",
	),
	(
		"voice_note",
		"the owner's voice notes sent home by the phone (paulgsc/server#460), game bites, Studio takes, soundbites, grammar-lens takes and LeetType spoken answers included, each described: what it was said for and which one, when it was recorded and reached home, its container, size and hash; see docs/identity.md invariant 23",
	),
	(
		"voice_note_audio",
		"the audio of each of the owner's voice notes, byte for byte as the phone sent it; see docs/identity.md invariant 23",
	),
	(
		"voice_transcript",
		"what home heard in each of the owner's voice notes, every attempt: the model and engine, what was heard (or that nothing was said, or why it could not be heard), the language and its probability, the audio's length, and when; see docs/identity.md invariant 23",
	),
	(
		"voice_transcript_word",
		"each word home heard in the owner's voice notes (paulgsc/some-ui GWB11), per transcript that heard something: the word as heard, its place, and when it began and ended in the note; see docs/identity.md invariant 23",
	),
	(
		"drama_prompt",
		"each next scene's prompt the owner's phone asked for and sent home (paulgsc/some-ui#2179): the prompt's markdown as written, which holds the scene asked for and the last drama's review in the owner's words, its stamp, and when it reached home; see docs/identity.md invariant 31",
	),
	(
		"written_reflection",
		"each written reflection the owner's phone sent home (paulgsc/some-ui#2178): the typed line as typed, the session it is about, when it was written and how far into the session, and when it reached home; see docs/identity.md invariant 29",
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
///
/// Nothing about the person, the request, or the time goes into it. The
/// owner's is no different: which account is the owner's is read from the
/// `account` table (`auth::owner`), never from the id.
///
/// Rows written before auth existed carry `subject-local`, a placeholder no
/// account holds now that the one-time claim that could take them over is
/// retired (#510); they are ordinary rows under a subject nobody can sign in
/// as.
#[must_use]
pub fn new_account_subject() -> String {
	let mut bytes = [0_u8; 16];
	rand::rng().fill_bytes(&mut bytes);
	String::from("subject-") + &hex::encode(bytes)
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

/// A paired phone's request, which a browser's session answers `403` here.
///
/// The mirror of [`BrowserSubject`]: the session must be a phone's bearer
/// token. It reads the session `SubjectId` resolved, as both read the bearer
/// first (`SessionToken::from_headers`).
#[derive(Debug)]
pub struct Paired {
	pub subject: SubjectId,
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
		match SessionToken::from_headers(&parts.headers) {
			Some((_, Carrier::Bearer)) => Ok(Self { subject }),
			_ => Err(FileHostError::Forbidden),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::new_account_subject;

	#[test]
	fn a_new_subject_is_random_and_names_nothing() {
		let a = new_account_subject();
		let b = new_account_subject();
		assert_ne!(a, b);
		assert_eq!(a.len(), "subject-".len() + 32);
		assert!(a.strip_prefix("subject-").unwrap().bytes().all(|byte| byte.is_ascii_hexdigit()));
	}
}
