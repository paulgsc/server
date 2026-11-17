//! Passkey accounts and sign-in sessions.
//!
//! Storage only. This crate stores three things per account — a subject id,
//! the passkeys that open it, and hashes of the sessions a sign-in handed
//! out — and has no opinion about WebAuthn, cookies, or who a subject is.
//! Verifying a passkey is `file_host`'s job; so is deciding what a new
//! account's subject id should be. Everything here takes already-decided
//! values and stores them.
//!
//! Two things it does own, because they are about how much a subject can
//! store rather than about auth:
//!
//! - [`MAX_PASSKEYS_PER_SUBJECT`]: a new passkey past it is refused, never
//!   silently evicted.
//! - [`MAX_SESSIONS_PER_SUBJECT`]: a new browser session past it ends the
//!   browser session closest to expiring, so a subject signed in on many
//!   browsers still owns a bounded number of rows.
//! - [`MAX_PHONES_PER_SUBJECT`]: a paired phone is never evicted by either
//!   cap; a new pairing past it is refused, like a passkey.
//!
//! No method here writes a timestamp of its own. Session expiry is an instant
//! the caller computed, so a test can hand in a fixed `now`, and no column
//! records when an account was made or last used. The one creation time is a
//! paired phone's day (`auth_session.paired_on`), which the caller computes
//! too (some-ui#3092; docs/identity.md invariant 10).

pub mod repository;

pub use repository::{Accounts, AddPasskey, Admission, AuthRepository, NewPhone, PairPhone, PairedPhone, SessionRow, StoredPasskey};

/// How many passkeys one account may hold. A person with a phone, a laptop
/// and a hardware key in two ecosystems needs a handful; this is generous
/// above that rather than tuned tight.
pub const MAX_PASSKEYS_PER_SUBJECT: i64 = 16;

/// How many live browser sessions one account may hold at once.
///
/// One per browser the person signs in on; past this, the browser session
/// closest to expiring ends. Paired phones are not counted and never ended
/// by it.
pub const MAX_SESSIONS_PER_SUBJECT: i64 = 32;

/// How many paired phones one account may hold at once.
///
/// Past this, a new pairing is refused until the owner signs one out, never
/// evicted: a phone that home silently forgot is what this cap exists to
/// prevent (C491).
pub const MAX_PHONES_PER_SUBJECT: i64 = 16;
