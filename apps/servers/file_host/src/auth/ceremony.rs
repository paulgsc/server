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

/// The handle a client holds between `start` and `finish`. Not a secret: it
/// only finds the stored challenge, and finishing still needs the
/// authenticator's signature over that challenge.
pub(crate) type CeremonyId = [u8; 16];

#[derive(Default)]
pub(crate) struct CeremonyStore {
	open: Mutex<HashMap<CeremonyId, (Instant, Ceremony)>>,
}

impl CeremonyStore {
	/// Open a ceremony, or return `None` when [`MAX_OPEN_CEREMONIES`] are
	/// already open and unexpired.
	pub(crate) fn open(&self, ceremony: Ceremony, now: Instant) -> Option<CeremonyId> {
		let mut id = CeremonyId::default();
		rand::rng().fill_bytes(&mut id);
		let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
		if open.len() >= MAX_OPEN_CEREMONIES {
			open.retain(|_, (expires, _)| *expires > now);
			if open.len() >= MAX_OPEN_CEREMONIES {
				return None;
			}
		}
		open.insert(id, (now + CEREMONY_TTL, ceremony));
		drop(open);
		Some(id)
	}

	/// Take a ceremony out, once. An expired one is dropped and reads as
	/// absent.
	pub(crate) fn take(&self, id: &CeremonyId, now: Instant) -> Option<Ceremony> {
		let (expires, ceremony) = self.open.lock().unwrap_or_else(PoisonError::into_inner).remove(id)?;
		(expires > now).then_some(ceremony)
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
