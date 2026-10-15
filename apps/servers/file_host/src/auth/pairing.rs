//! Pairing codes in flight: how a phone gets a session without a passkey.
//!
//! The Android app cannot hold the session cookie (its pages are served from
//! `https://localhost`, another site) and cannot run a passkey ceremony for
//! this relying party. So a browser that is already signed in asks for a
//! short code (`start`), the person types it on the phone, and the phone
//! trades it for a session token of its own (`finish`), which it sends as
//! `Authorization: Bearer` ([`super::cookie::SessionToken::from_headers`]).
//!
//! A code is a secret while it is open. It lives here, in memory, for at most
//! [`PAIRING_TTL`], is found by its digest, and is taken out exactly once: the
//! same shape as `ceremony`, for the same reasons. One subject holds at most
//! one open code; asking again replaces it. [`MAX_OPEN_PAIRINGS`] bounds the
//! store across every subject.
//!
//! 8 characters of Crockford base32 is 40 bits. At the default per-client
//! limit on `/api/v1` (`RATE_LIMIT`, 60 a minute), one client guessing one
//! open code inside its ten minutes has about one chance in 1.8 billion; N
//! clients, or N open codes, multiply that by N. Sign-out everywhere and
//! account deletion close the account's open code ([`PairingStore::forget`]).
//!
//! The phone calls these routes, and everything after, over native HTTP: a
//! request with no `Origin` or `Sec-Fetch-Site` passes `auth::csrf`, which a
//! `WebView` `fetch` from `https://localhost` would not.

use crate::redacted::Redacted;
use rand::Rng;
use sha2::{Digest, Sha256};
use std::{
	collections::HashMap,
	sync::{Mutex, PoisonError},
	time::{Duration, Instant},
};

/// How long a code stays open: time to read it off one screen and type it on
/// another.
pub const PAIRING_TTL: Duration = Duration::from_secs(600);

/// [`PAIRING_TTL`] in the seconds a response's `expiresAt` is counted in.
pub const PAIRING_TTL_SECONDS: i64 = 600;

/// How many codes may be open at once, across every subject.
pub const MAX_OPEN_PAIRINGS: usize = 10_000;

/// Crockford base32: no I, L, O or U, so nothing typed reads two ways.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Characters in a code.
pub const CODE_LENGTH: usize = 8;

/// A code as the person reads and types it. Never logged, never stored.
pub(crate) struct PairingCode(Redacted<String>);

impl PairingCode {
	fn mint() -> Self {
		let mut rng = rand::rng();
		let code = (0..CODE_LENGTH).map(|_| char::from(ALPHABET[rng.random_range(0..ALPHABET.len())])).collect();
		Self(Redacted::new(code))
	}

	/// What a person typed, read the way Crockford base32 reads it: case,
	/// spaces and hyphens ignored, `I`/`L` as `1` and `O` as `0`. `None` when
	/// it cannot be a code.
	pub(crate) fn typed(text: &str) -> Option<Self> {
		let code: String = text
			.chars()
			.filter(|c| !c.is_whitespace() && *c != '-')
			.map(|c| match c.to_ascii_uppercase() {
				'I' | 'L' => '1',
				'O' => '0',
				other => other,
			})
			.collect();
		(code.len() == CODE_LENGTH && code.bytes().all(|byte| ALPHABET.contains(&byte))).then(|| Self(Redacted::new(code)))
	}

	/// The code to show the person who asked for it.
	pub(crate) fn expose(&self) -> &str {
		self.0.expose()
	}

	fn digest(&self) -> [u8; 32] {
		Sha256::digest(self.0.expose().as_bytes()).into()
	}
}

#[derive(Default)]
pub(crate) struct PairingStore {
	/// By the code's digest: the subject it pairs to, and when it expires.
	open: Mutex<HashMap<[u8; 32], (Instant, String)>>,
}

impl PairingStore {
	/// A new code for `subject`, replacing any it already holds. `None` when
	/// [`MAX_OPEN_PAIRINGS`] are open and unexpired.
	pub(crate) fn open(&self, subject: &str, now: Instant) -> Option<PairingCode> {
		let code = PairingCode::mint();
		let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
		open.retain(|_, (expires, owner)| *expires > now && owner != subject);
		if open.len() >= MAX_OPEN_PAIRINGS {
			return None;
		}
		open.insert(code.digest(), (now + PAIRING_TTL, subject.to_owned()));
		drop(open);
		Some(code)
	}

	/// Close `subject`'s open code, if it has one: a sign-out everywhere or an
	/// account deletion must not leave a session to be had after it.
	pub(crate) fn forget(&self, subject: &str) {
		self.open.lock().unwrap_or_else(PoisonError::into_inner).retain(|_, (_, owner)| owner != subject);
	}

	/// The subject `code` pairs to, once. An expired code is dropped and
	/// reads as absent.
	pub(crate) fn take(&self, code: &PairingCode, now: Instant) -> Option<String> {
		let (expires, subject) = self.open.lock().unwrap_or_else(PoisonError::into_inner).remove(&code.digest())?;
		(expires > now).then_some(subject)
	}
}

#[cfg(test)]
mod tests {
	use super::{PairingCode, PairingStore, CODE_LENGTH, MAX_OPEN_PAIRINGS, PAIRING_TTL, PAIRING_TTL_SECONDS};
	use std::time::Instant;

	#[test]
	fn both_spellings_of_the_lifetime_agree() {
		assert_eq!(i64::try_from(PAIRING_TTL.as_secs()).unwrap(), PAIRING_TTL_SECONDS);
	}

	#[test]
	fn a_code_is_taken_once_and_not_after_it_expires() {
		let store = PairingStore::default();
		let now = Instant::now();

		let code = store.open("subject-a", now).unwrap();
		assert_eq!(code.expose().len(), CODE_LENGTH);
		assert_eq!(store.take(&code, now).as_deref(), Some("subject-a"));
		assert!(store.take(&code, now).is_none(), "a redeemed code cannot be replayed");

		let code = store.open("subject-a", now).unwrap();
		assert!(store.take(&code, now + PAIRING_TTL).is_none(), "an expired code reads as absent");
	}

	#[test]
	fn asking_again_replaces_the_subjects_open_code_only() {
		let store = PairingStore::default();
		let now = Instant::now();
		let first = store.open("subject-a", now).unwrap();
		let other = store.open("subject-b", now).unwrap();
		let second = store.open("subject-a", now).unwrap();

		assert!(store.take(&first, now).is_none(), "the older code no longer pairs");
		assert_eq!(store.take(&second, now).as_deref(), Some("subject-a"));
		assert_eq!(store.take(&other, now).as_deref(), Some("subject-b"));
	}

	#[test]
	fn forgetting_a_subject_closes_its_code_only() {
		let store = PairingStore::default();
		let now = Instant::now();
		let mine = store.open("subject-a", now).unwrap();
		let theirs = store.open("subject-b", now).unwrap();
		store.forget("subject-a");
		assert!(store.take(&mine, now).is_none());
		assert_eq!(store.take(&theirs, now).as_deref(), Some("subject-b"));
	}

	#[test]
	fn a_typed_code_reads_the_way_crockford_base32_does() {
		let store = PairingStore::default();
		let now = Instant::now();
		let code = store.open("subject-a", now).unwrap();
		let shown = code.expose().to_owned();
		let typed = shown.to_ascii_lowercase().chars().enumerate().fold(String::new(), |mut out, (n, c)| {
			if n == 4 {
				out.push('-');
			}
			out.push(c);
			out
		});
		assert_eq!(store.take(&PairingCode::typed(&typed).unwrap(), now).as_deref(), Some("subject-a"));

		assert_eq!(PairingCode::typed(" o1il-OOOO\n").unwrap().expose(), "01110000");
		for wrong in ["", "1234567", "123456789", "1234567U", "12345678!"] {
			assert!(PairingCode::typed(wrong).is_none(), "{wrong}");
		}
	}

	#[test]
	fn a_full_store_refuses_until_something_expires() {
		let store = PairingStore::default();
		let now = Instant::now();
		{
			let mut open = store.open.lock().unwrap();
			for n in 0..MAX_OPEN_PAIRINGS {
				let mut digest = [0_u8; 32];
				digest[..8].copy_from_slice(&n.to_le_bytes());
				open.insert(digest, (now + PAIRING_TTL, String::from("subject-") + &n.to_string()));
			}
		}
		assert!(store.open("subject-new", now).is_none(), "full, and nothing has expired");
		assert!(store.open("subject-new", now + PAIRING_TTL).is_some(), "expired codes make room");
	}
}
