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
//! - [`MAX_SESSIONS_PER_SUBJECT`]: a new session past it ends the one closest
//!   to expiring, so a subject signed in on many devices still owns a bounded
//!   number of rows.
//!
//! No method here writes a timestamp of its own. Session expiry is an instant
//! the caller computed, so a test can hand in a fixed `now`, and no column
//! records when an account was made or last used.

pub mod repository;

pub use repository::{AddPasskey, AuthRepository, SessionRow, StoredPasskey};

/// How many passkeys one account may hold. A person with a phone, a laptop
/// and a hardware key in two ecosystems needs a handful; this is generous
/// above that rather than tuned tight.
pub const MAX_PASSKEYS_PER_SUBJECT: i64 = 16;

/// How many live sessions one account may hold at once. One per browser the
/// person signs in on; past this, the session closest to expiring ends.
pub const MAX_SESSIONS_PER_SUBJECT: i64 = 32;
