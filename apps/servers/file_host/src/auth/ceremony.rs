//! Passkey ceremonies in flight.
//!
//! A ceremony is two requests: `start` hands the browser a challenge, `finish`
//! checks what the authenticator signed. The state that pairs them (the
//! challenge, above all) must stay on the server, or a replayed `finish` could
//! reuse it. It lives here, in memory, for at most [`CEREMONY_TTL`], and is
//! taken out exactly once.
//!
//! In memory rather than in a table: a ceremony lasts a minute or two, a
//! restart costs only the ones in flight (the browser starts again), and
//! nothing about it is worth a row. Bounded by [`MAX_OPEN_CEREMONIES`], so a
//! client that only ever calls `start` cannot grow the process without limit.
//!
//! The store itself, [`OneTimeStore`], holds anything with the same shape: a
//! short life, taken exactly once, bounded. OAuth's pending approvals and its
//! authorization codes use it too (`auth::oauth`).

use rand::RngCore;
use std::{
	collections::HashMap,
	sync::{Mutex, PoisonError},
	time::{Duration, Instant},
};
use webauthn_rs::prelude::{DiscoverableAuthentication, PasskeyRegistration, Uuid};

/// How long a ceremony stays open. Five minutes is the WebAuthn default
/// timeout the options already ask the browser for, plus a little slack for
/// the round trip.
pub const CEREMONY_TTL: Duration = Duration::from_secs(330);

/// How many ceremonies may be open at once, across every client.
pub const MAX_OPEN_CEREMONIES: usize = 10_000;

pub(crate) enum Ceremony {
	/// Creating a passkey. `subject` is `None` for a new account, and the
	/// signed-in subject when adding a passkey to an existing one.
	Register {
		state: PasskeyRegistration,
		user_handle: Uuid,
		subject: Option<String>,
	},
	SignIn {
		state: DiscoverableAuthentication,
	},
}

/// The handle a client holds for something in a [`OneTimeStore`]: 16 random
/// bytes. For a ceremony it is not a secret: it only finds the stored
/// challenge, and finishing still needs the authenticator's signature over
/// that challenge. An OAuth authorization code is one of these, and is.
pub(crate) type OneTimeId = [u8; 16];

/// The handle a client holds between `start` and `finish`.
pub(crate) type CeremonyId = OneTimeId;

/// Values that live for `ttl` at most, are taken out exactly once, and number
/// at most `capacity` unexpired at a time.
pub(crate) struct OneTimeStore<T> {
	open: Mutex<HashMap<OneTimeId, (Instant, T)>>,
	ttl: Duration,
	capacity: usize,
}

/// The passkey ceremonies in flight.
pub(crate) type CeremonyStore = OneTimeStore<Ceremony>;

impl Default for CeremonyStore {
	fn default() -> Self {
		Self::new(CEREMONY_TTL, MAX_OPEN_CEREMONIES)
	}
}

impl<T> OneTimeStore<T> {
	pub(crate) fn new(ttl: Duration, capacity: usize) -> Self {
		Self {
			open: Mutex::new(HashMap::new()),
			ttl,
			capacity,
		}
	}

	/// Store a value under a fresh random id, or return `None` when
	/// `capacity` are already stored and unexpired.
	pub(crate) fn open(&self, value: T, now: Instant) -> Option<OneTimeId> {
		let mut id = OneTimeId::default();
		rand::rng().fill_bytes(&mut id);
		let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
		if open.len() >= self.capacity {
			open.retain(|_, (expires, _)| *expires > now);
			if open.len() >= self.capacity {
				return None;
			}
		}
		open.insert(id, (now + self.ttl, value));
		drop(open);
		Some(id)
	}

	/// Take a value out, once. An expired one is dropped and reads as absent.
	pub(crate) fn take(&self, id: &OneTimeId, now: Instant) -> Option<T> {
		let (expires, value) = self.open.lock().unwrap_or_else(PoisonError::into_inner).remove(id)?;
		(expires > now).then_some(value)
	}
}

#[cfg(test)]
mod tests {
	use super::{Ceremony, CeremonyStore, CEREMONY_TTL, MAX_OPEN_CEREMONIES};
	use std::time::Instant;
	use webauthn_rs::prelude::{Url, WebauthnBuilder};

	fn sign_in() -> Ceremony {
		let origin = Url::parse("https://app.test").unwrap();
		let relying_party = WebauthnBuilder::new("app.test", &origin).unwrap().build().unwrap();
		let (_, state) = relying_party.start_discoverable_authentication().unwrap();
		Ceremony::SignIn { state }
	}

	#[test]
	fn a_ceremony_is_taken_once_and_not_after_it_expires() {
		let store = CeremonyStore::default();
		let now = Instant::now();

		let id = store.open(sign_in(), now).unwrap();
		assert!(store.take(&id, now).is_some());
		assert!(store.take(&id, now).is_none(), "a finished ceremony cannot be replayed");

		let id = store.open(sign_in(), now).unwrap();
		assert!(store.take(&id, now + CEREMONY_TTL).is_none(), "an expired ceremony reads as absent");
	}

	#[test]
	fn a_full_store_refuses_until_something_expires() {
		let store = CeremonyStore::default();
		let now = Instant::now();
		let ceremony = sign_in();
		let Ceremony::SignIn { state } = ceremony else { unreachable!() };
		{
			let mut open = store.open.lock().unwrap();
			for n in 0..MAX_OPEN_CEREMONIES {
				let mut id = [0_u8; 16];
				id[..8].copy_from_slice(&n.to_le_bytes());
				open.insert(id, (now + CEREMONY_TTL, Ceremony::SignIn { state: state.clone() }));
			}
		}
		assert!(store.open(sign_in(), now).is_none(), "full, and nothing has expired");
		assert!(store.open(sign_in(), now + CEREMONY_TTL).is_some(), "expired ceremonies make room");
	}
}
